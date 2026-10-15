//! Exercise the injected Linux skill using the CI runner's Node and shell tools.
//! Python and PowerShell are deliberately unavailable to the executor fixtures.

#[test]
fn injected_image_generation_works_with_shell_tools() {
    for args in [
        vec!["bash", "-n", "skills/proxy-imagegen/scripts/generate.sh"],
        vec!["shellcheck", "skills/proxy-imagegen/scripts/generate.sh"],
        vec!["shellcheck", "skills/proxy-imagegen/scripts/generate.ps1"],
        vec!["node", "--test", "tests/proxy_imagegen.mjs"],
    ] {
        let output = std::process::Command::new(args[0])
            .args(&args[1..])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("CI must provide Bash, shellcheck, curl, jq and Node");
        assert!(
            output.status.success(),
            "{} failed:\n{}\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
