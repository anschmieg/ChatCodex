use std::process::Command;

use codex_apply_patch::CODEX_CORE_APPLY_PATCH_ARG1;

#[test]
fn binary_supports_native_apply_patch_self_invocation() {
    let workspace = tempfile::tempdir().expect("workspace");
    let status = Command::new(env!("CARGO_BIN_EXE_codex-native-harness-mcp"))
        .arg(CODEX_CORE_APPLY_PATCH_ARG1)
        .arg("*** Begin Patch\n*** Add File: hello.txt\n+hello from native patch\n*** End Patch\n")
        .current_dir(workspace.path())
        .status()
        .expect("run native apply_patch mode");

    assert!(status.success());
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("hello.txt")).expect("patched file"),
        "hello from native patch\n"
    );
}
