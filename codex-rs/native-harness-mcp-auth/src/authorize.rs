//! Authorization-code flow: `/oauth/authorize` (Cloudflare Access gate +
//! consent) and `/oauth/authorize/decide` (mint the code and 302 back to
//! the client).

use askama::Template;
use axum::Form;
use axum::extract::Query;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Redirect;
use serde::Deserialize;

use crate::state::AuthState;
use crate::storage::AuthCodeRecord;
use crate::storage::new_opaque_token;
use crate::storage::now;

const CF_AUTHORIZATION_COOKIE: &str = "CF_Authorization";

#[derive(Debug, Deserialize)]
pub struct AuthorizeQuery {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    #[serde(default)]
    pub resource: Option<String>,
}

#[derive(Template)]
#[template(path = "consent.html")]
struct ConsentTemplate<'a> {
    client_id: &'a str,
    subject: &'a str,
    email: Option<&'a str>,
    scope: &'a str,
    redirect_uri: &'a str,
    state: &'a str,
    code_challenge: &'a str,
    code_challenge_method: &'a str,
    error: Option<&'a str>,
}

pub async fn authorize(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Query(query): Query<AuthorizeQuery>,
) -> impl IntoResponse {
    if query.response_type != "code" {
        return (
            StatusCode::BAD_REQUEST,
            "unsupported response_type; only 'code' is supported",
        )
            .into_response();
    }
    let client = match state.store().get_client(&query.client_id) {
        Ok(Some(client)) => client,
        Ok(None) => return (StatusCode::BAD_REQUEST, "unknown client_id").into_response(),
        Err(error) => return error_response(error),
    };
    if !client
        .redirect_uris
        .iter()
        .any(|uri| uri == &query.redirect_uri)
    {
        return (StatusCode::BAD_REQUEST, "redirect_uri not registered").into_response();
    }
    let method = query.code_challenge_method.as_deref().unwrap_or("");
    let code_challenge_method = method;
    if query
        .code_challenge
        .as_deref()
        .is_none_or(|_| method != "S256")
    {
        return (
            StatusCode::BAD_REQUEST,
            "code_challenge and code_challenge_method=S256 are required",
        )
            .into_response();
    }

    // Pull the CF_Authorization cookie. If absent, redirect to the team's
    // login endpoint with a `next` pointing back to this authorize URL.
    let token = headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|raw| raw.split(';'))
        .find_map(|pair| {
            let mut iter = pair.trim().splitn(2, '=');
            match (iter.next(), iter.next()) {
                (Some(name), Some(value)) if name == CF_AUTHORIZATION_COOKIE => {
                    Some(value.to_string())
                }
                _ => None,
            }
        });

    let Some(token) = token else {
        let next = build_authorize_url(&state, &query);
        let login = state.cf().login_url(&next);
        return Redirect::to(&login).into_response();
    };

    let claims = match state.cf().verify(&token).await {
        Ok(Some(claims)) => claims,
        Ok(None) => {
            let next = build_authorize_url(&state, &query);
            let login = state.cf().login_url(&next);
            return Redirect::to(&login).into_response();
        }
        Err(error) => return error_response(error),
    };

    let template = ConsentTemplate {
        client_id: &query.client_id,
        subject: &claims.sub,
        email: claims.email.as_deref(),
        scope: query.scope.as_deref().unwrap_or("mcp:tools"),
        redirect_uri: &query.redirect_uri,
        state: query.state.as_deref().unwrap_or(""),
        code_challenge: query.code_challenge.as_deref().unwrap_or(""),
        code_challenge_method,
        error: None,
    };
    match template.render() {
        Ok(body) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
            body,
        )
            .into_response(),
        Err(error) => error_response(anyhow::anyhow!(error)),
    }
}

#[derive(Debug, Deserialize)]
pub struct DecideForm {
    pub decision: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub state: Option<String>,
    pub code_challenge: String,
    pub code_challenge_method: String,
}

pub async fn decide(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Form(form): Form<DecideForm>,
) -> impl IntoResponse {
    let token = match extract_cf_cookie(&headers) {
        Some(token) => token,
        None => {
            return Redirect::to(&state.cf().login_url("/oauth/authorize")).into_response();
        }
    };
    let claims = match state.cf().verify(&token).await {
        Ok(Some(claims)) => claims,
        _ => {
            return Redirect::to(&state.cf().login_url("/oauth/authorize")).into_response();
        }
    };

    if form.decision != "allow" {
        let mut redirect =
            url::Url::parse(&form.redirect_uri).unwrap_or_else(|_error| parse_https_localhost());
        redirect
            .query_pairs_mut()
            .append_pair("error", "access_denied");
        redirect.query_pairs_mut().append_pair("iss", &state.config().issuer());
        if let Some(state_value) = &form.state {
            redirect.query_pairs_mut().append_pair("state", state_value);
        }
        return Redirect::to(redirect.as_str()).into_response();
    }

    let client = match state.store().get_client(&form.client_id) {
        Ok(Some(client)) => client,
        _ => return (StatusCode::BAD_REQUEST, "unknown client_id").into_response(),
    };
    if !client
        .redirect_uris
        .iter()
        .any(|uri| uri == &form.redirect_uri)
    {
        return (StatusCode::BAD_REQUEST, "redirect_uri not registered").into_response();
    }
    let code = new_opaque_token();
    let code_hash = crate::storage::hash_token(&code);
    let record = AuthCodeRecord {
        code_hash,
        client_id: form.client_id.clone(),
        subject: claims.sub,
        redirect_uri: form.redirect_uri.clone(),
        scope: form.scope.clone(),
        code_challenge: Some(form.code_challenge.clone()),
        code_challenge_method: Some(form.code_challenge_method.clone()),
        expires_at: now() + 60,
        consumed_at: None,
    };
    if let Err(error) = state.store().insert_auth_code(&record) {
        return error_response(error);
    }

    let mut redirect =
        url::Url::parse(&form.redirect_uri).unwrap_or_else(|_error| parse_https_localhost());
    redirect.query_pairs_mut().append_pair("code", &code);
    redirect.query_pairs_mut().append_pair("iss", &state.config().issuer());
    if let Some(state_value) = &form.state {
        redirect.query_pairs_mut().append_pair("state", state_value);
    }
    Redirect::to(redirect.as_str()).into_response()
}

fn extract_cf_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|raw| raw.split(';'))
        .find_map(|pair| {
            let mut iter = pair.trim().splitn(2, '=');
            match (iter.next(), iter.next()) {
                (Some(name), Some(value)) if name == CF_AUTHORIZATION_COOKIE => {
                    Some(value.to_string())
                }
                _ => None,
            }
        })
}

fn build_authorize_url(state: &AuthState, query: &AuthorizeQuery) -> String {
    let mut url = url::Url::parse(&state.config().authorization_endpoint())
        .unwrap_or_else(|_| parse_https_localhost());
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("response_type", &query.response_type);
        pairs.append_pair("client_id", &query.client_id);
        pairs.append_pair("redirect_uri", &query.redirect_uri);
        if let Some(scope) = &query.scope {
            pairs.append_pair("scope", scope);
        }
        if let Some(state_value) = &query.state {
            pairs.append_pair("state", state_value);
        }
        if let Some(challenge) = &query.code_challenge {
            pairs.append_pair("code_challenge", challenge);
        }
        if let Some(method) = &query.code_challenge_method {
            pairs.append_pair("code_challenge_method", method);
        }
    }
    url.to_string()
}

fn error_response(error: impl std::fmt::Display) -> axum::response::Response {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
}

#[allow(clippy::expect_used)]
fn parse_https_localhost() -> url::Url {
    url::Url::parse("https://localhost/").expect("https://localhost/ must parse")
}
