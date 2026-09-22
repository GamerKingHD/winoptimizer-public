fn main() {
    let violations = winoptimizer_lib::capabilities::audit_capability_manifest();
    if !violations.is_empty() {
        eprintln!("capability manifest audit failed: {violations:?}");
        std::process::exit(1);
    }
    let manifest = winoptimizer_lib::capabilities::capability_descriptors();
    println!(
        "{}",
        serde_json::to_string_pretty(manifest).expect("capability manifest is serializable")
    );
}
