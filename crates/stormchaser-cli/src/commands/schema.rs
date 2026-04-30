use anyhow::{Context, Result};
use clap::Subcommand;
use stormchaser_dsl::hcl_schema::json_schema_to_hcl;
use stormchaser_model::schema_gen::generate_dsl_schema;

/// CLI subcommands for managing schemas.
#[derive(Subcommand)]
pub enum SchemaCommands {
    /// Generate schema for the DSL
    Generate {
        /// Output format (json, hcl)
        #[arg(short, long, default_value = "json")]
        format: String,
    },
}

/// Handles the `schema` command logic.
pub fn handle(command: SchemaCommands) -> Result<()> {
    match command {
        SchemaCommands::Generate { format } => {
            let schema = generate_dsl_schema();

            if format.to_lowercase() == "hcl" {
                let json_val = serde_json::to_value(&schema)?;
                let hcl_body =
                    json_schema_to_hcl(&json_val).context("Failed to serialize schema to HCL")?;
                println!("{}", hcl::to_string(&hcl_body)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&schema)?);
            }
        }
    }
    Ok(())
}
