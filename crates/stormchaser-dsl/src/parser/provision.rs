use anyhow::{Context, Result};
use hcl::{Block, Body};
use stormchaser_model::dsl;

use super::expr::expr_to_string;

/// Parses a provision sub-block in the new syntax:
/// `provision { <resource_type> "name" { ... } }`
pub fn parse_provision_sub_block(prov_block: &Block) -> Result<dsl::Provision> {
    let resource_type = prov_block.identifier().to_string();
    let name = prov_block
        .labels()
        .first()
        .map(|l| l.as_str().to_string())
        .context("Provision sub-block must have a name label")?;
    parse_provision_attributes(name, resource_type, prov_block.body())
}

/// Parses a provision block in the legacy syntax:
/// `provision "name" { resource_type = "download" ... }`
pub fn parse_provision_legacy_block(block: &Block) -> Result<dsl::Provision> {
    let name = block
        .labels()
        .first()
        .map(|l| l.as_str().to_string())
        .context("Legacy provision block must have a name label")?;
    let mut resource_type = "download".to_string();
    for attr in block.body().attributes() {
        if attr.key() == "resource_type" {
            resource_type = expr_to_string(attr.expr())?;
        }
    }
    parse_provision_attributes(name, resource_type, block.body())
}

/// Extracts the common provision fields from a block body, for both syntaxes.
fn parse_provision_attributes(
    name: String,
    resource_type: String,
    body: &Body,
) -> Result<dsl::Provision> {
    let mut source = None;
    let mut url = None;
    let mut destination = "/".to_string();
    let mut mode = None;
    let mut checksum = None;
    let mut from = None;

    for attr in body.attributes() {
        match attr.key() {
            "source" => source = Some(expr_to_string(attr.expr())?),
            "url" => url = Some(expr_to_string(attr.expr())?),
            "destination" => destination = expr_to_string(attr.expr())?,
            "mode" => mode = Some(expr_to_string(attr.expr())?),
            "checksum" => checksum = Some(expr_to_string(attr.expr())?),
            "from" => from = Some(expr_to_string(attr.expr())?),
            _ => {}
        }
    }

    Ok(dsl::Provision {
        name,
        resource_type,
        source,
        url,
        destination,
        mode,
        checksum,
        from,
    })
}
