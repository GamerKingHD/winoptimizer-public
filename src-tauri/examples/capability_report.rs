use serde::Serialize;
use std::fmt::Write as _;

use winoptimizer_lib::capabilities::{
    all_descriptors, audit_capability_manifest, audit_registered_commands, AuditViolation,
    CommandDescriptor,
};

fn main() {
    let command_violations = audit_registered_commands();
    let capability_violations = audit_capability_manifest();
    if command_violations.is_empty() && capability_violations.is_empty() {
        let descriptors = current_descriptors();
        print!("{}", render_report(&descriptors));
        return;
    }

    report_violations("registered command", &command_violations);
    report_violations("capability manifest", &capability_violations);
    std::process::exit(1);
}

fn current_descriptors() -> Vec<CommandDescriptor> {
    let mut descriptors = all_descriptors();
    descriptors.sort_unstable_by(|left, right| left.command.cmp(right.command));
    descriptors
}

fn report_violations(scope: &str, violations: &[AuditViolation]) {
    if violations.is_empty() {
        return;
    }

    eprintln!("{scope} audit failed:");
    for violation in violations {
        eprintln!("- {}: {}", violation.command, violation.message);
    }
}

fn render_report(descriptors: &[CommandDescriptor]) -> String {
    let mut report = String::from(
        "# Current source-backed command capability audit\n\n\
This deterministic inventory is generated from the current Rust capability registry.\n\
It contains descriptor-backed metadata only; VM, test, and frontend evidence are not\n\
asserted here.\n\n\
| command | classification | mutates | kernel | capability_id | operation_id | proof | risk | recovery | safety_flags |\n\
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n",
    );

    for descriptor in descriptors {
        let operation_id = descriptor.operation_id.as_deref().unwrap_or("none");
        let classification = serialized(&descriptor.kind);
        let proof = serialized(&descriptor.proof_status);
        let risk = serialized(&descriptor.risk);
        let recovery = serialized(&descriptor.recovery);
        let safety_flags = format!(
            "confirmation={}; revalidation={}; verification={}; journal={}",
            descriptor.requires_confirmation,
            descriptor.requires_revalidation,
            descriptor.requires_verification,
            descriptor.requires_journal,
        );

        writeln!(
            report,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            markdown_cell(descriptor.command),
            markdown_cell(&classification),
            descriptor.mutates_system,
            descriptor.uses_operation_kernel,
            markdown_cell(&descriptor.capability_id),
            markdown_cell(operation_id),
            markdown_cell(&proof),
            markdown_cell(&risk),
            markdown_cell(&recovery),
            markdown_cell(&safety_flags),
        )
        .expect("writing to a String cannot fail");
    }

    report
}

fn serialized<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .expect("registry enum must serialize")
        .as_str()
        .expect("registry enum must serialize as a string")
        .to_owned()
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_is_sorted_and_contains_only_descriptor_backed_columns() {
        let descriptors = current_descriptors();
        let report = render_report(&descriptors);
        let table_rows = report.lines().filter(|line| line.starts_with('|')).count();

        assert_eq!(table_rows, descriptors.len() + 2);
        assert!(report.contains(
            "| command | classification | mutates | kernel | capability_id | operation_id | proof | risk | recovery | safety_flags |"
        ));
        assert!(report.contains("| execute_appx_plan | plan_executor | true | true |"));
        assert!(
            report.find("| create_appx_plan |").unwrap()
                < report.find("| scan_cleanup_targets |").unwrap()
        );
        assert!(!report.contains("current_tests"));
        assert!(!report.contains("vm_acceptance"));
        assert!(!report.contains("frontend_route"));
    }
}
