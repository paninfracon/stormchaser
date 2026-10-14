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
        /// The token. Prefer --token-stdin: other users on the host can read a
        /// command's arguments, and the shell keeps them in its history.
        #[arg(long, conflicts_with = "token_stdin")]
        token: Option<String>,
        /// Read the token from the first line of standard input.
        #[arg(long)]
        token_stdin: bool,
    },
    /// Switch the active profile
    Switch { name: String },
    /// Remove a profile
    Remove { name: String },
    /// Show the current active profile
    Current,
}

/// Reads a token from the first line of `input`.
fn read_token(mut input: impl std::io::BufRead) -> Result<String> {
    let mut line = String::new();
    input
        .read_line(&mut line)
        .context("Could not read the token from standard input")?;
    let token = line.trim();
    if token.is_empty() {
        anyhow::bail!("No token on standard input");
    }
    Ok(token.to_string())
}

fn config_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("net", "paninfracon", "stormchaser")
        .context("Could not determine config directory")?;
    let config_dir = dirs.config_dir();
    fs::create_dir_all(config_dir)?;
    restrict_to_owner(config_dir, 0o700)?;
    Ok(config_dir.join("profiles.json"))
}

/// Profiles hold bearer tokens, so only the owner may read them.
#[cfg(unix)]
fn restrict_to_owner(path: &std::path::Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("Could not restrict permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &std::path::Path, _mode: u32) -> Result<()> {
    Ok(())
}

/// Writes `content` to `path` so that it is never readable by anyone but the owner, not
/// even between creation and a later chmod.
fn write_private(path: &std::path::Path, content: &str) -> Result<()> {
    use std::io::Write;

    let tmp = path.with_extension("json.tmp");
    let _ = fs::remove_file(&tmp);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp)
        .with_context(|| format!("Could not create {}", tmp.display()))?;
    file.write_all(content.as_bytes())?;
    file.sync_all()?;
    fs::rename(&tmp, path)?;
    Ok(())
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
    write_private(&path, &content)
}

/// Stores a token from a login against the profile for `url`, creating one if there is
/// none, and returns the profile's name.
pub fn store_token(url: &str, token: &str) -> Result<String> {
    let mut config = load_profiles()?;
    let name = assign_token(&mut config, url, token);
    save_profiles(&config)?;
    Ok(name)
}

fn assign_token(config: &mut ProfilesConfig, url: &str, token: &str) -> String {
    let same_url =
        |profile: &Profile| profile.url.trim_end_matches('/') == url.trim_end_matches('/');
    let existing = config
        .active_profile
        .clone()
        .filter(|active| config.profiles.get(active).is_some_and(same_url))
        .or_else(|| {
            let mut names: Vec<_> = config
                .profiles
                .iter()
                .filter(|(_, profile)| same_url(profile))
                .map(|(name, _)| name.clone())
                .collect();
            names.sort();
            names.into_iter().next()
        });
    let name = existing.unwrap_or_else(|| {
        let mut name = "default".to_string();
        let mut n = 2;
        while config.profiles.contains_key(&name) {
            name = format!("default-{n}");
            n += 1;
        }
        name
    });
    config.profiles.insert(
        name.clone(),
        Profile {
            url: url.to_string(),
            token: Some(token.to_string()),
        },
    );
    // The profile just logged in to is the one the next command should use
    config.active_profile = Some(name.clone());
    name
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
        ProfilesCommands::Add {
            name,
            url,
            token,
            token_stdin,
        } => {
            let token = if token_stdin {
                Some(read_token(std::io::stdin().lock())?)
            } else {
                token
            };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_read_from_the_first_line_of_input() {
        assert_eq!(read_token(&b"tok-en\nrest"[..]).unwrap(), "tok-en");
        assert_eq!(read_token(&b"  tok \r\n"[..]).unwrap(), "tok");
        assert!(read_token(&b"\n"[..]).is_err());
        assert!(read_token(&b""[..]).is_err());
    }

    fn profile(url: &str) -> Profile {
        Profile {
            url: url.to_string(),
            token: None,
        }
    }

    #[test]
    fn a_login_token_goes_to_the_profile_for_that_url() {
        let mut config = ProfilesConfig::default();
        config
            .profiles
            .insert("prod".into(), profile("https://prod.example.com"));
        config
            .profiles
            .insert("dev".into(), profile("http://localhost:3000/"));
        config.active_profile = Some("prod".into());

        let name = assign_token(&mut config, "http://localhost:3000", "t1");
        assert_eq!(name, "dev");
        assert_eq!(config.active_profile.as_deref(), Some("dev"));
        assert_eq!(config.profiles["dev"].token.as_deref(), Some("t1"));
        // The other environment's profile never sees this token
        assert_eq!(config.profiles["prod"].token, None);
    }

    #[test]
    fn a_login_to_a_new_url_creates_a_profile_without_touching_others() {
        let mut config = ProfilesConfig::default();
        config
            .profiles
            .insert("default".into(), profile("https://prod.example.com"));

        let name = assign_token(&mut config, "https://staging.example.com", "t2");
        assert_eq!(name, "default-2");
        assert_eq!(config.profiles["default"].url, "https://prod.example.com");
        assert_eq!(config.profiles["default"].token, None);
    }

    #[cfg(unix)]
    #[test]
    fn the_profile_file_is_private_from_the_moment_it_exists() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        // A world-readable file left by an older version is replaced, not reused
        fs::write(&path, "{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        write_private(&path, "{\"profiles\":{}}").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"profiles\":{}}");
    }
}
