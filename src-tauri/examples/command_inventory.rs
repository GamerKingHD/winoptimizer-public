fn main() {
    let violations = winoptimizer_lib::capabilities::audit_registered_commands();
    if !violations.is_empty() {
        eprintln!("registered command audit failed: {violations:?}");
        std::process::exit(1);
    }

    let descriptors = winoptimizer_lib::capabilities::all_descriptors();
    println!(
        "{}",
        serde_json::to_string_pretty(&descriptors).expect("command descriptors are serializable")
    );
}
