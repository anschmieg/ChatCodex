#!/usr/bin/env python3
"""Export Hermes Honcho memory and optionally ingest it into Hindsight.

Run this from the Hermes Agent virtual environment:

    ~/.hermes/hermes-agent/.venv/bin/python migrate_honcho_to_hindsight.py
    ~/.hermes/hermes-agent/.venv/bin/python migrate_honcho_to_hindsight.py --apply

The default is export/preview only. Re-running with --apply is idempotent because
stable Hindsight document IDs are retained with update_mode="replace".
"""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import inspect
import json
import os
import re
import sys
import time
import urllib.parse
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable


def load_simple_env(path: Path) -> None:
    if not path.exists():
        return
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        os.environ.setdefault(key.strip(), value.strip())


def json_value(value: Any) -> Any:
    if dataclasses.is_dataclass(value):
        return dataclasses.asdict(value)
    if hasattr(value, "model_dump"):
        return value.model_dump(mode="json")
    if isinstance(value, datetime):
        return value.isoformat()
    if isinstance(value, Path):
        return str(value)
    if isinstance(value, dict):
        return {str(key): json_value(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [json_value(item) for item in value]
    if hasattr(value, "__dict__"):
        return {
            key: json_value(item)
            for key, item in vars(value).items()
            if not key.startswith("_")
        }
    return value


def redact_config(value: Any) -> Any:
    secret_pattern = re.compile(r"(api.?key|token|secret|password)", re.IGNORECASE)
    if isinstance(value, dict):
        return {
            key: "<redacted>" if secret_pattern.search(str(key)) else redact_config(item)
            for key, item in value.items()
        }
    if isinstance(value, list):
        return [redact_config(item) for item in value]
    return value


def safe_id(value: str, max_length: int = 96) -> str:
    normalized = re.sub(r"[^A-Za-z0-9_.-]+", "-", value).strip("-")
    if normalized and len(normalized) <= max_length:
        return normalized
    digest = hashlib.sha256(value.encode("utf-8")).hexdigest()[:16]
    prefix = normalized[: max_length - 17] if normalized else "item"
    return f"{prefix}-{digest}"


def iso_timestamp(value: Any) -> str:
    if isinstance(value, datetime):
        return value.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")
    return str(value or "")


def role_for_peer(peer_id: str, user_peer: str, ai_peer: str) -> str:
    if peer_id == user_peer:
        return "User"
    if peer_id == ai_peer:
        return "Assistant"
    return f"Peer {peer_id}"


def render_session(session: Any, messages: list[Any], user_peer: str, ai_peer: str) -> str:
    session_created_at = getattr(session, "created_at", None)
    if session_created_at is None and messages:
        session_created_at = getattr(messages[0], "created_at", None)
    lines = [
        "# Imported Honcho conversation",
        "",
        f"Session ID: {session.id}",
        f"Created: {iso_timestamp(session_created_at)}",
        f"User peer: {user_peer}",
        f"Assistant peer: {ai_peer}",
        "",
    ]
    for message in messages:
        role = role_for_peer(message.peer_id, user_peer, ai_peer)
        lines.extend(
            [
                f"## {role} ({iso_timestamp(message.created_at)})",
                "",
                message.content,
                "",
            ]
        )
    return "\n".join(lines).strip() + "\n"


def render_conclusions(
    conclusions: list[Any], observer: str, observed: str, title: str
) -> str:
    lines = [
        f"# {title}",
        "",
        f"Observer peer: {observer}",
        f"Observed peer: {observed}",
        "",
    ]
    for conclusion in conclusions:
        session_id = getattr(conclusion, "session_id", None) or "global"
        lines.extend(
            [
                f"- [{iso_timestamp(conclusion.created_at)}] {conclusion.content}",
                f"  Source: Honcho conclusion {conclusion.id}; session {session_id}",
            ]
        )
    return "\n".join(lines).strip() + "\n"


def render_profile(
    user_peer: str,
    ai_peer: str,
    user_card: list[str],
    ai_about_user_card: list[str],
    user_representation: str,
    ai_about_user_representation: str,
) -> str:
    sections = [
        "# Imported Honcho profile",
        "",
        f"User peer: {user_peer}",
        f"Assistant peer: {ai_peer}",
        "",
        "## User peer card",
        "",
        *(f"- {item}" for item in user_card),
        "",
        "## Assistant's card about the user",
        "",
        *(f"- {item}" for item in ai_about_user_card),
        "",
        "## User representation",
        "",
        user_representation or "(not available)",
        "",
        "## Assistant's representation of the user",
        "",
        ai_about_user_representation or "(not available)",
        "",
    ]
    return "\n".join(sections).strip() + "\n"


def collect_page(page: Iterable[Any]) -> list[Any]:
    return list(page)


def call_compatible(method: Any, /, *args: Any, **kwargs: Any) -> Any:
    """Call an SDK method using only keyword arguments it supports."""
    try:
        parameters = inspect.signature(method).parameters.values()
    except (TypeError, ValueError):
        return method(*args, **kwargs)
    accepts_any = any(
        parameter.kind == inspect.Parameter.VAR_KEYWORD
        for parameter in parameters
    )
    if accepts_any:
        return method(*args, **kwargs)
    supported = {
        parameter.name
        for parameter in parameters
        if parameter.kind
        in {
            inspect.Parameter.POSITIONAL_OR_KEYWORD,
            inspect.Parameter.KEYWORD_ONLY,
        }
    }
    return method(*args, **{key: value for key, value in kwargs.items() if key in supported})


def write_json(path: Path, value: Any) -> None:
    path.write_text(
        json.dumps(json_value(value), indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )


def read_json_url(url: str, timeout: int = 30) -> dict[str, Any] | None:
    try:
        with urllib.request.urlopen(url, timeout=timeout) as response:
            return json.load(response)
    except urllib.error.HTTPError as exc:
        if exc.code == 404:
            return None
        raise


def daemon_api_url(client: Any, path: str) -> str:
    return f"{client._manager.get_url(client.profile).rstrip('/')}{path}"


def document_status(
    client: Any, bank_id: str, document_id: str, content: str
) -> tuple[str, dict[str, Any] | None]:
    url = daemon_api_url(
        client,
        f"/v1/default/banks/{urllib.parse.quote(bank_id, safe='')}/documents/"
        f"{urllib.parse.quote(document_id, safe='')}",
    )
    document = read_json_url(url)
    if document is None:
        return "absent", None
    if document.get("original_text") != content:
        return "different", document
    return "complete", document


def operation_status(
    client: Any, bank_id: str, operation_id: str, *, include_payload: bool = False
) -> dict[str, Any] | None:
    suffix = "?include_payload=true" if include_payload else ""
    url = daemon_api_url(
        client,
        f"/v1/default/banks/{urllib.parse.quote(bank_id, safe='')}/operations/"
        f"{urllib.parse.quote(operation_id, safe='')}{suffix}",
    )
    return read_json_url(url)


def find_active_operation(
    client: Any, bank_id: str, document_id: str
) -> str | None:
    encoded_bank = urllib.parse.quote(bank_id, safe="")
    for status_name in ("processing", "pending"):
        query = urllib.parse.urlencode(
            {"status": status_name, "type": "retain", "limit": 100}
        )
        url = daemon_api_url(
            client, f"/v1/default/banks/{encoded_bank}/operations?{query}"
        )
        listing = read_json_url(url) or {}
        for item in listing.get("operations", []):
            operation_id = item.get("operation_id") or item.get("id")
            if not operation_id:
                continue
            detail = operation_status(
                client, bank_id, operation_id, include_payload=True
            )
            payload = (detail or {}).get("task_payload")
            if payload and document_id in json.dumps(payload, ensure_ascii=False):
                return str(operation_id)
    return None


def format_duration(seconds: float) -> str:
    seconds = max(0, int(seconds))
    minutes, seconds = divmod(seconds, 60)
    hours, minutes = divmod(minutes, 60)
    if hours:
        return f"{hours}h {minutes:02d}m {seconds:02d}s"
    if minutes:
        return f"{minutes}m {seconds:02d}s"
    return f"{seconds}s"


def load_checkpoint(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {"version": 1, "documents": {}}
    return json.loads(path.read_text(encoding="utf-8"))


def save_checkpoint(path: Path, checkpoint: dict[str, Any]) -> None:
    checkpoint["updated_at"] = datetime.now(timezone.utc).isoformat()
    write_json(path, checkpoint)


def resolve_hindsight_client(hermes_home: Path):
    config_path = hermes_home / "hindsight" / "config.json"
    config = json.loads(config_path.read_text(encoding="utf-8"))
    if config.get("mode") not in {"local", "local_embedded"}:
        raise RuntimeError(
            f"Expected local_embedded Hindsight mode in {config_path}, "
            f"found {config.get('mode')!r}"
        )

    from hindsight import HindsightEmbedded

    provider = str(config.get("llm_provider") or "")
    base_url = config.get("llm_base_url")
    if provider in {"openrouter", "openai_compatible"}:
        provider = "openai"
    return (
        HindsightEmbedded(
            profile=str(config.get("profile") or "hermes"),
            llm_provider=provider,
            llm_api_key=os.environ.get("HINDSIGHT_LLM_API_KEY", ""),
            llm_model=str(config.get("llm_model") or ""),
            llm_base_url=str(base_url) if base_url else None,
            idle_timeout=0,
        ),
        str(config.get("bank_id") or "hermes"),
    )


def retain_document(
    client: Any,
    bank_id: str,
    document_id: str,
    content: str,
    context: str,
    tags: list[str],
    timestamp: datetime | None = None,
    *,
    checkpoint: dict[str, Any],
    checkpoint_path: Path,
    position: int,
    total: int,
) -> str:
    content_digest = hashlib.sha256(content.encode("utf-8")).hexdigest()
    records = checkpoint.setdefault("documents", {})
    record = records.setdefault(document_id, {})
    started = time.monotonic()

    remote_state, remote_document = document_status(
        client, bank_id, document_id, content
    )
    if remote_state == "complete":
        record.update(
            {
                "status": "completed",
                "content_sha256": content_digest,
                "source": "remote_document",
                "memory_unit_count": remote_document.get("memory_unit_count"),
            }
        )
        save_checkpoint(checkpoint_path, checkpoint)
        print(
            f"[{position}/{total}] SKIP {document_id}: already complete "
            f"({remote_document.get('memory_unit_count')} memory units)",
            flush=True,
        )
        return "skipped"
    if remote_state == "different":
        raise RuntimeError(
            f"Hindsight document {document_id} already exists with different content. "
            "Refusing to replace it automatically."
        )

    operation_ids = list(record.get("operation_ids") or [])
    if not operation_ids:
        recovered_operation = find_active_operation(client, bank_id, document_id)
        if recovered_operation:
            operation_ids = [recovered_operation]
            record.update(
                {
                    "status": "processing",
                    "content_sha256": content_digest,
                    "operation_ids": operation_ids,
                    "source": "recovered_operation",
                }
            )
            save_checkpoint(checkpoint_path, checkpoint)
            print(
                f"[{position}/{total}] RESUME {document_id}: recovered active "
                f"operation {recovered_operation}",
                flush=True,
            )

    if operation_ids:
        statuses = [
            operation_status(client, bank_id, operation_id)
            for operation_id in operation_ids
        ]
        if all(status and status.get("status") == "completed" for status in statuses):
            remote_state, remote_document = document_status(
                client, bank_id, document_id, content
            )
            if remote_state == "complete":
                record.update(
                    {
                        "status": "completed",
                        "content_sha256": content_digest,
                        "memory_unit_count": remote_document.get("memory_unit_count"),
                    }
                )
                save_checkpoint(checkpoint_path, checkpoint)
                print(
                    f"[{position}/{total}] SKIP {document_id}: checkpoint operation "
                    "already completed",
                    flush=True,
                )
                return "skipped"
        if any(
            status and status.get("status") in {"pending", "processing"}
            for status in statuses
        ):
            print(
                f"[{position}/{total}] RESUME {document_id}: waiting for existing "
                f"operation",
                flush=True,
            )
        else:
            operation_ids = []

    if not operation_ids:
        print(
            f"[{position}/{total}] SUBMIT {document_id} "
            f"({len(content):,} characters)",
            flush=True,
        )
        response = client.retain(
            bank_id=bank_id,
            content=content,
            timestamp=timestamp,
            context=context,
            document_id=document_id,
            metadata={"source": "honcho", "migration_version": "1"},
            tags=tags,
            update_mode="replace",
            retain_async=True,
        )
        operation_ids = list(getattr(response, "operation_ids", None) or [])
        operation_id = getattr(response, "operation_id", None)
        if operation_id and operation_id not in operation_ids:
            operation_ids.append(operation_id)
        if not operation_ids:
            raise RuntimeError(
                f"Hindsight did not return an operation ID for document {document_id}"
            )
        record.update(
            {
                "status": "submitted",
                "content_sha256": content_digest,
                "operation_ids": operation_ids,
                "submitted_at": datetime.now(timezone.utc).isoformat(),
            }
        )
        save_checkpoint(checkpoint_path, checkpoint)

    for current_operation_id in operation_ids:
        deadline = time.monotonic() + 3600
        last_status = None
        last_report = 0.0
        while True:
            status = operation_status(client, bank_id, current_operation_id)
            if status is None:
                raise RuntimeError(
                    f"Hindsight operation {current_operation_id} disappeared"
                )
            current_status = status["status"]
            now = time.monotonic()
            if current_status != last_status or now - last_report >= 15:
                progress = status.get("progress") or {}
                progress_text = ""
                if progress:
                    progress_text = f", progress={json.dumps(progress, separators=(',', ':'))}"
                print(
                    f"  [{position}/{total}] {document_id}: {current_status}, "
                    f"elapsed={format_duration(now - started)}{progress_text}",
                    flush=True,
                )
                last_status = current_status
                last_report = now
            record["status"] = current_status
            record["last_operation_status"] = status
            save_checkpoint(checkpoint_path, checkpoint)
            if current_status == "completed":
                break
            if current_status in {"failed", "cancelled", "not_found"}:
                detail = status.get("error_message") or current_status
                record["error"] = detail
                save_checkpoint(checkpoint_path, checkpoint)
                raise RuntimeError(
                    f"Hindsight operation {current_operation_id} for "
                    f"{document_id} ended with {current_status}: {detail}"
                )
            if time.monotonic() >= deadline:
                raise TimeoutError(
                    f"Hindsight operation {current_operation_id} for "
                    f"{document_id} did not finish within one hour"
                )
            time.sleep(2)

    remote_state, remote_document = document_status(
        client, bank_id, document_id, content
    )
    if remote_state != "complete":
        raise RuntimeError(
            f"Hindsight reported completion for {document_id}, but the completed "
            "document could not be verified."
        )
    record.update(
        {
            "status": "completed",
            "content_sha256": content_digest,
            "completed_at": datetime.now(timezone.utc).isoformat(),
            "memory_unit_count": remote_document.get("memory_unit_count"),
        }
    )
    save_checkpoint(checkpoint_path, checkpoint)
    print(
        f"[{position}/{total}] DONE {document_id}: "
        f"{remote_document.get('memory_unit_count')} memory units in "
        f"{format_duration(time.monotonic() - started)}",
        flush=True,
    )
    return "completed"


def run_import(
    hindsight: Any,
    bank_id: str,
    archive_dir: Path,
    profile_document_id: str,
    profile_content: str,
    conclusion_documents: list[tuple[str, str]],
    session_documents: list[tuple[str, str, datetime | None]],
) -> dict[str, int]:
    documents: list[tuple[str, str, str, list[str], datetime | None]] = [
        (
            profile_document_id,
            profile_content,
            "Profile and durable user-model data imported from Honcho",
            ["source:honcho", "kind:profile"],
            None,
        )
    ]
    documents.extend(
        (
            document_id,
            content,
            "Derived conclusions imported from Honcho",
            ["source:honcho", "kind:conclusions"],
            None,
        )
        for document_id, content in conclusion_documents
    )
    documents.extend(
        (
            document_id,
            content,
            "Conversation transcript imported from Honcho",
            ["source:honcho", "kind:conversation"],
            created_at,
        )
        for document_id, content, created_at in session_documents
    )

    checkpoint_path = archive_dir / "migration-state.json"
    checkpoint = load_checkpoint(checkpoint_path)
    checkpoint["bank_id"] = bank_id
    checkpoint["total_documents"] = len(documents)
    save_checkpoint(checkpoint_path, checkpoint)

    counts = {"completed": 0, "skipped": 0}
    overall_started = time.monotonic()
    print(
        f"Migration plan: {len(documents)} documents; checkpoint: {checkpoint_path}",
        flush=True,
    )
    for position, (document_id, content, context, tags, timestamp) in enumerate(
        documents, start=1
    ):
        result = retain_document(
            hindsight,
            bank_id,
            document_id,
            content,
            context,
            tags,
            timestamp,
            checkpoint=checkpoint,
            checkpoint_path=checkpoint_path,
            position=position,
            total=len(documents),
        )
        counts[result] += 1
        processed = counts["completed"] + counts["skipped"]
        elapsed = time.monotonic() - overall_started
        average = elapsed / processed
        remaining = average * (len(documents) - processed)
        print(
            f"Progress: {processed}/{len(documents)} "
            f"({processed / len(documents):.0%}), "
            f"new={counts['completed']}, skipped={counts['skipped']}, "
            f"elapsed={format_duration(elapsed)}, "
            f"estimated_remaining={format_duration(remaining)}",
            flush=True,
        )
    return counts


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--apply",
        action="store_true",
        help="Ingest the exported documents into Hindsight. Without this flag, only export.",
    )
    parser.add_argument("--peer", help="Honcho user peer ID; defaults to Hermes peerName.")
    parser.add_argument(
        "--bank-id", help="Override the Hindsight bank ID from its Hermes configuration."
    )
    parser.add_argument(
        "--since",
        help="Only migrate sessions created on/after this ISO date, for example 2026-01-01.",
    )
    parser.add_argument(
        "--max-sessions",
        type=int,
        help="Limit migration to the newest N matching sessions.",
    )
    parser.add_argument(
        "--archive-dir",
        type=Path,
        help=(
            "Export destination. Defaults under ~/.hermes/migrations/. "
            "With --apply, an existing completed preview archive is reused."
        ),
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    os.umask(0o077)
    hermes_home = Path(os.environ.get("HERMES_HOME", "~/.hermes")).expanduser()
    load_simple_env(hermes_home / ".env")

    try:
        from plugins.memory.honcho.client import (
            HonchoClientConfig,
            get_honcho_client,
        )
    except ImportError as exc:
        print(
            "Run this script with ~/.hermes/hermes-agent/.venv/bin/python "
            "from the Hermes Agent checkout.",
            file=sys.stderr,
        )
        raise SystemExit(2) from exc

    config = HonchoClientConfig.from_global_config()
    ai_peer = config.ai_peer
    user_peer = args.peer or config.peer_name
    client = get_honcho_client(config)

    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    archive_dir = args.archive_dir or (
        hermes_home / "migrations" / "honcho-to-hindsight" / timestamp
    )
    if archive_dir.exists():
        if not args.apply:
            print(f"Archive already exists: {archive_dir}", file=sys.stderr)
            return 2
        manifest_path = archive_dir / "manifest.json"
        if not manifest_path.exists():
            print(f"Existing archive has no manifest: {archive_dir}", file=sys.stderr)
            return 2
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        if manifest.get("user_peer") != user_peer:
            print(
                f"Archive user peer {manifest.get('user_peer')!r} does not match "
                f"{user_peer!r}.",
                file=sys.stderr,
            )
            return 2
        if manifest.get("applied"):
            print(f"Archive is already marked as applied: {archive_dir}")
            return 0
        print(f"Reusing preview archive: {archive_dir}")
        session_documents = []
        for session_info in manifest.get("sessions", []):
            path = Path(session_info["path"])
            if not path.is_absolute():
                path = archive_dir / path
            created_text = session_info.get("created_at")
            created_at = (
                datetime.fromisoformat(created_text.replace("Z", "+00:00"))
                if created_text
                else None
            )
            session_documents.append(
                (session_info["document_id"], path.read_text(encoding="utf-8"), created_at)
            )
        conclusion_documents = [
            (
                document_id,
                (archive_dir / "documents" / f"{document_id}.md").read_text(
                    encoding="utf-8"
                ),
            )
            for document_id in manifest.get("conclusion_documents", [])
        ]
        profile_document_id = manifest["profile_document"]
        profile_content = (
            archive_dir / "documents" / f"{profile_document_id}.md"
        ).read_text(encoding="utf-8")

        hindsight, configured_bank_id = resolve_hindsight_client(hermes_home)
        bank_id = args.bank_id or configured_bank_id
        print(f"Ingesting into Hindsight bank: {bank_id}")
        try:
            counts = run_import(
                hindsight,
                bank_id,
                archive_dir,
                profile_document_id,
                profile_content,
                conclusion_documents,
                session_documents,
            )
        finally:
            hindsight.close()
        manifest["applied"] = True
        manifest["bank_id"] = bank_id
        manifest["completed_at"] = datetime.now(timezone.utc).isoformat()
        manifest["migration_counts"] = counts
        write_json(manifest_path, manifest)
        print("Migration completed successfully.")
        print(f"Backup retained at: {archive_dir}")
        return 0

    archive_dir.mkdir(parents=True, exist_ok=False)
    documents_dir = archive_dir / "documents"
    documents_dir.mkdir()

    write_json(archive_dir / "honcho-config.redacted.json", redact_config(config.raw))
    peers = collect_page(call_compatible(client.peers, size=100))
    write_json(archive_dir / "peers.json", peers)

    if not user_peer or user_peer == ai_peer:
        aliases = sorted(
            peer
            for peer in set(config.user_peer_aliases.values())
            if peer and peer != ai_peer
        )
        if len(aliases) == 1:
            user_peer = aliases[0]
    if not user_peer or user_peer == ai_peer:
        peer_ids = sorted(
            str(getattr(peer, "id", ""))
            for peer in peers
            if getattr(peer, "id", None) and getattr(peer, "id", None) != ai_peer
        )
        if len(peer_ids) == 1:
            user_peer = peer_ids[0]
        else:
            print(f"Honcho workspace: {config.workspace_id}")
            print(f"Assistant peer:   {ai_peer}")
            print(f"Available peers:  {', '.join(peer_ids) or '(none besides assistant)'}")
            print(f"Partial export:   {archive_dir}")
            print(
                "Could not safely infer the user peer. Re-run with --peer PEER_ID.",
                file=sys.stderr,
            )
            return 2

    print(f"Honcho workspace: {config.workspace_id}")
    print(f"User peer:        {user_peer}")
    print(f"Assistant peer:   {ai_peer}")
    print(f"Export directory: {archive_dir}")

    user = client.peer(user_peer)
    assistant = client.peer(ai_peer)
    sessions = collect_page(call_compatible(user.sessions, size=100, reverse=True))

    if args.since:
        since = datetime.fromisoformat(args.since.replace("Z", "+00:00"))
        if since.tzinfo is None:
            since = since.replace(tzinfo=timezone.utc)
        sessions = [
            session
            for session in sessions
            if getattr(session, "created_at", None) is not None
            and getattr(session, "created_at") >= since
        ]
    sessions.sort(
        key=lambda session: getattr(session, "created_at", None)
        or datetime.min.replace(tzinfo=timezone.utc),
        reverse=True,
    )
    if args.max_sessions is not None:
        sessions = sessions[: args.max_sessions]

    exported_sessions: list[dict[str, Any]] = []
    session_documents: list[tuple[str, str, datetime | None]] = []
    total_messages = 0
    for index, session in enumerate(sessions, start=1):
        messages = collect_page(call_compatible(session.messages, size=100))
        messages.sort(
            key=lambda message: getattr(message, "created_at", None)
            or datetime.min.replace(tzinfo=timezone.utc)
        )
        session_created_at = getattr(session, "created_at", None)
        if session_created_at is None and messages:
            session_created_at = getattr(messages[0], "created_at", None)
        total_messages += len(messages)
        document_id = f"honcho-session-{safe_id(session.id)}"
        content = render_session(session, messages, user_peer, ai_peer)
        path = documents_dir / f"{document_id}.md"
        path.write_text(content, encoding="utf-8")
        write_json(
            archive_dir / f"session-{index:05d}-{safe_id(session.id)}.json",
            {"session": session, "messages": messages},
        )
        exported_sessions.append(
            {
                "id": session.id,
                "created_at": iso_timestamp(session_created_at),
                "message_count": len(messages),
                "document_id": document_id,
                "path": str(path),
            }
        )
        session_documents.append((document_id, content, session_created_at))

    scopes = [
        (
            "assistant-about-user",
            assistant.conclusions_of(user_peer),
            ai_peer,
            user_peer,
            "Honcho conclusions: assistant about user",
        ),
        (
            "user-self",
            user.conclusions,
            user_peer,
            user_peer,
            "Honcho conclusions: user self-model",
        ),
    ]
    conclusion_documents: list[tuple[str, str]] = []
    conclusion_export: dict[str, Any] = {}
    for name, scope, observer, observed, title in scopes:
        conclusions = collect_page(call_compatible(scope.list, size=100))
        conclusion_export[name] = conclusions
        if conclusions:
            content = render_conclusions(conclusions, observer, observed, title)
            document_id = f"honcho-conclusions-{name}"
            (documents_dir / f"{document_id}.md").write_text(content, encoding="utf-8")
            conclusion_documents.append((document_id, content))
    write_json(archive_dir / "conclusions.json", conclusion_export)

    def best_effort(call, default):
        try:
            return call()
        except Exception as exc:  # Export should survive optional API failures.
            print(f"Warning: optional Honcho profile export failed: {exc}", file=sys.stderr)
            return default

    user_card = best_effort(lambda: user.get_card() or [], [])
    ai_about_user_card = best_effort(lambda: assistant.get_card(target=user_peer) or [], [])
    user_representation = best_effort(lambda: user.representation(), "")
    ai_about_user_representation = best_effort(
        lambda: assistant.representation(target=user_peer), ""
    )
    profile_content = render_profile(
        user_peer,
        ai_peer,
        user_card,
        ai_about_user_card,
        user_representation,
        ai_about_user_representation,
    )
    profile_document_id = "honcho-profile"
    (documents_dir / f"{profile_document_id}.md").write_text(
        profile_content, encoding="utf-8"
    )
    write_json(
        archive_dir / "profile.json",
        {
            "user_card": user_card,
            "assistant_about_user_card": ai_about_user_card,
            "user_representation": user_representation,
            "assistant_about_user_representation": ai_about_user_representation,
        },
    )

    manifest = {
        "created_at": datetime.now(timezone.utc).isoformat(),
        "workspace_id": config.workspace_id,
        "user_peer": user_peer,
        "assistant_peer": ai_peer,
        "session_count": len(exported_sessions),
        "message_count": total_messages,
        "sessions": exported_sessions,
        "conclusion_documents": [item[0] for item in conclusion_documents],
        "profile_document": profile_document_id,
        "applied": False,
    }
    write_json(archive_dir / "manifest.json", manifest)

    print(f"Exported {len(exported_sessions)} sessions and {total_messages} messages.")
    print(f"Prepared {len(session_documents) + len(conclusion_documents) + 1} documents.")
    if not args.apply:
        print("Preview complete. Review the archive, then re-run with --apply.")
        return 0

    hindsight, configured_bank_id = resolve_hindsight_client(hermes_home)
    bank_id = args.bank_id or configured_bank_id
    print(f"Ingesting into Hindsight bank: {bank_id}")
    try:
        counts = run_import(
            hindsight,
            bank_id,
            archive_dir,
            profile_document_id,
            profile_content,
            conclusion_documents,
            session_documents,
        )
    finally:
        hindsight.close()

    manifest["applied"] = True
    manifest["bank_id"] = bank_id
    manifest["completed_at"] = datetime.now(timezone.utc).isoformat()
    manifest["migration_counts"] = counts
    write_json(archive_dir / "manifest.json", manifest)
    print("Migration completed successfully.")
    print(f"Backup retained at: {archive_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
