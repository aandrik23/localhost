use std::fs;
use std::path::Path;

use super::model::Config;
use super::validation::validate_config;

pub fn parse_config_str(
    input: &str,
) -> Result<Config, String> {
    let config: Config =
        toml::from_str(input)
            .map_err(|err| {
                format!(
                    "invalid TOML configuration: {}",
                    err
                )
            })?;

    validate_config(&config)?;

    Ok(config)
}

pub fn load_config<P>(
    path: P,
) -> Result<Config, String>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();

    let contents =
        fs::read_to_string(path)
            .map_err(|err| {
                format!(
                    "failed to read configuration '{}': {}",
                    path.display(),
                    err
                )
            })?;

    parse_config_str(&contents)
}