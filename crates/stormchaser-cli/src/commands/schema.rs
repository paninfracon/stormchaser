use anyhow::{Context, Result};
use clap::{Subcommand, ValueEnum};
use stormchaser_dsl::hcl_schema::json_schema_to_hcl;
use stormchaser_model::schema_gen::{generate_dsl_schema, generate_event_schemas};

/// Output format for schema generation.
#[derive(ValueEnum, Clone, Debug)]
pub enum SchemaFormat {
    /// JSON Schema (Draft 7)
    Json,
    /// HCL format
    Hcl,
}

/// CLI subcommands for managing schemas.
#[derive(Subcommand)]
pub enum SchemaCommands {
    /// Generate schema for the DSL
    Generate {
        /// Output format
        #[arg(short, long, value_enum, default_value_t = SchemaFormat::Json)]
        format: SchemaFormat,
    },
    /// Generate schemas for NATS events and output them to a directory
    GenerateEvents {
        /// Output directory
        #[arg(short, long, default_value = "schemas")]
        output_dir: String,
    },
}

/// Handles the `schema` command logic.
pub fn handle(command: SchemaCommands) -> Result<()> {
    match command {
        SchemaCommands::Generate { format } => {
            let schema = generate_dsl_schema();

            match format {
                SchemaFormat::Hcl => {
                    let json_val = serde_json::to_value(&schema)?;
                    let hcl_body = json_schema_to_hcl(&json_val)
                        .context("Failed to serialize schema to HCL")?;
                    println!("{}", hcl::to_string(&hcl_body)?);
                }
                SchemaFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&schema)?);
                }
            }
        }
        SchemaCommands::GenerateEvents { output_dir } => {
            std::fs::create_dir_all(&output_dir)?;
            let schemas = generate_event_schemas();
            for (name, schema) in schemas {
                let filename = format!("{}/{}.json", output_dir, name);
                let json_str = serde_json::to_string_pretty(&schema)?;
                std::fs::write(&filename, json_str)?;
                println!("✓ Wrote schema to {}", filename);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_schema_generate_json() {
        let cmd = SchemaCommands::Generate {
            format: SchemaFormat::Json,
        };
        let result = handle(cmd);
        result.unwrap();
    }

    #[test]
    fn test_handle_schema_generate_hcl() {
        let cmd = SchemaCommands::Generate {
            format: SchemaFormat::Hcl,
        };
        let result = handle(cmd);
        result.unwrap();
    }
}
