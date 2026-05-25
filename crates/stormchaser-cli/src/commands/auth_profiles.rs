use anyhow::{Context, Result};
use clap::Subcommand;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct ProfilesConfig {
    pub active_profile: Option<String>,
    pub profiles: HashMap<String, Profile>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Profile {
    pub url: String,
    pub token: Option<String>,
}

#[derive(Subcommand)]
pub enum ProfilesCommands {
    /// List all configured profiles
    List,
    /// Add or update a profile
    Add {
        name: String,
        #[arg(long)]
        url: String,
        #[arg(long)]
        token: Option<String>,
    },
    /// Switch the active profile
    Switch { name: String },
    /// Remove a profile
    Remove { name: String },
    /// Show the current active profile
    Current,
}

fn config_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("net", "paninfracon", "stormchaser")
        .context("Could not determine config directory")?;
    let config_dir = dirs.config_dir();
    fs::create_dir_all(config_dir)?;
    Ok(config_dir.join("profiles.json"))
}

pub fn load_profiles() -> Result<ProfilesConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(ProfilesConfig::default());
    }
    let content = fs::read_to_string(path)?;
    let config = serde_json::from_str(&content).unwrap_or_default();
    Ok(config)
}

pub fn save_profiles(config: &ProfilesConfig) -> Result<()> {
    let path = config_path()?;
    let content = serde_json::to_string_pretty(config)?;
    fs::write(path, content)?;
    Ok(())
}

pub fn get_active_profile() -> Result<Option<Profile>> {
    let config = load_profiles()?;
    if let Some(active) = config.active_profile {
        if let Some(profile) = config.profiles.get(&active) {
            return Ok(Some(profile.clone()));
        }
    }
    Ok(None)
}

pub async fn handle(command: ProfilesCommands) -> Result<()> {
    match command {
        ProfilesCommands::List => {
            let config = load_profiles()?;
            if config.profiles.is_empty() {
                println!("No profiles configured.");
            } else {
                for (name, profile) in &config.profiles {
                    let active_marker = if Some(name.clone()) == config.active_profile {
                        "*"
                    } else {
                        " "
                    };
                    println!("{} {} (URL: {})", active_marker, name, profile.url);
                }
            }
        }
        ProfilesCommands::Add { name, url, token } => {
            let mut config = load_profiles()?;
            config.profiles.insert(
                name.clone(),
                Profile {
                    url,
                    token: token.clone(),
                },
            );
            if config.active_profile.is_none() {
                config.active_profile = Some(name.clone());
            }
            save_profiles(&config)?;
            println!("Profile '{}' added.", name);
        }
        ProfilesCommands::Switch { name } => {
            let mut config = load_profiles()?;
            if !config.profiles.contains_key(&name) {
                anyhow::bail!("Profile '{}' not found.", name);
            }
            config.active_profile = Some(name.clone());
            save_profiles(&config)?;
            println!("Switched to profile '{}'.", name);
        }
        ProfilesCommands::Remove { name } => {
            let mut config = load_profiles()?;
            if config.profiles.remove(&name).is_some() {
                if config.active_profile == Some(name.clone()) {
                    config.active_profile = None;
                }
                save_profiles(&config)?;
                println!("Profile '{}' removed.", name);
            } else {
                println!("Profile '{}' not found.", name);
            }
        }
        ProfilesCommands::Current => {
            let config = load_profiles()?;
            if let Some(active) = config.active_profile {
                if let Some(profile) = config.profiles.get(&active) {
                    println!("Active Profile: {}", active);
                    println!("URL: {}", profile.url);
                    if profile.token.is_some() {
                        println!("Token: [SET]");
                    } else {
                        println!("Token: [NOT SET]");
                    }
                } else {
                    println!("Active profile '{}' is missing from config.", active);
                }
            } else {
                println!("No active profile set.");
            }
        }
    }
    Ok(())
}
