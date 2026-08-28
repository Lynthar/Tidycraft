//! `tidycraft explain` — one rule's documentation, resolved from the id or its
//! family prefix.

use crate::CliError;
use std::process::ExitCode;
use tidycraft_core::analyzer::rule_docs;

pub fn run(rule_id: &str) -> Result<ExitCode, CliError> {
    let Some(doc) = rule_docs::rule_doc(rule_id) else {
        let families: Vec<&str> = rule_docs::RULE_DOCS.iter().map(|d| d.id).collect();
        return Err(CliError::Config(format!(
            "unknown rule `{rule_id}` — families: {}. Run `tidycraft rules` for every id.",
            families.join(", ")
        )));
    };
    println!("{rule_id} — {}", doc.title);
    println!();
    println!("{}", doc.summary);
    println!();
    println!(
        "full reference: docs/analyzer-rules.md, section \"{}\"",
        doc.section
    );
    Ok(ExitCode::SUCCESS)
}
