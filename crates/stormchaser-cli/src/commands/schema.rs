use anyhow::Result;
use clap::Subcommand;
use stormchaser_model::schema_gen::generate_dsl_schema;

#[derive(Subcommand)]
pub enum SchemaCommands {
    /// Generate JSON schema for the DSL
    Generate,
}

pub fn handle(command: SchemaCommands) -> Result<()> {
    match command {
        SchemaCommands::Generate => {
            let schema = generate_dsl_schema();
            println!("{}", serde_json::to_string_pretty(&schema)?);
        }
    }
    Ok(())
}
