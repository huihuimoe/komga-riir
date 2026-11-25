//! The OAuth2 login settings: the `spring.security.oauth2.client.*` providers, bound like Spring
//! Boot binds `OAuth2ClientProperties` from its map-valued keys, and how their users map to
//! Komga users.

use komga_oauth::{
    ClientSecret, OAuth2ClientProperties, ProviderProperties, RegistrationProperties,
};

use super::environment::Environment;
use super::property_file::PropertyFiles;
use super::{ConfigError, ConfigSource, Setting, default, lookup, parse_boolean};

const ACCOUNT_CREATION: &str = "komga.oauth2-account-creation";
const OIDC_EMAIL_VERIFICATION: &str = "komga.oidc-email-verification";
const FILE_PREFIX: &str = "spring.security.oauth2.client.";
const ENV_PREFIX: &str = "SPRING_SECURITY_OAUTH2_CLIENT_";

/// One property of a registration or provider, from any source.
struct Entry {
    kind: Kind,
    id: String,
    /// The relaxed property name: lowercase, without separators.
    property: String,
    /// The position of a `scope[n]` element.
    index: Option<usize>,
    value: String,
    key: String,
    origin: ConfigSource,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Registration,
    Provider,
}

/// Reads the registrations from the files, then lets the environment override them property by
/// property. Keys under the prefix that Spring would not bind are rejected.
/// The OAuth2 login settings.
#[derive(Debug, Clone)]
pub struct OAuth2Config {
    /// The providers, before their issuers are resolved.
    pub client: OAuth2ClientProperties,
    /// Create a user for an unknown email instead of refusing the login.
    pub account_creation: Setting<bool>,
    /// Refuse OpenID Connect logins whose email the provider has not verified.
    pub oidc_email_verification: Setting<bool>,
}

pub(super) fn resolve(
    env: &Environment,
    files: &mut PropertyFiles,
) -> Result<OAuth2Config, ConfigError> {
    let account_creation = lookup(env, files, ACCOUNT_CREATION)?
        .map_or(Ok(default(false)), |setting| {
            parse_boolean(ACCOUNT_CREATION, setting)
        })?;
    let oidc_email_verification = lookup(env, files, OIDC_EMAIL_VERIFICATION)?
        .map_or(Ok(default(true)), |setting| {
            parse_boolean(OIDC_EMAIL_VERIFICATION, setting)
        })?;
    Ok(OAuth2Config {
        client: resolve_client(env, files)?,
        account_creation,
        oidc_email_verification,
    })
}

fn resolve_client(
    env: &Environment,
    files: &mut PropertyFiles,
) -> Result<OAuth2ClientProperties, ConfigError> {
    let mut properties = OAuth2ClientProperties::default();
    let mut file_entries = Vec::new();
    for found in files.take_prefixed(FILE_PREFIX) {
        let origin = ConfigSource::File(found.path);
        let entry = file_entry(&found.key, found.value, origin.clone())
            .ok_or_else(|| unsupported(found.key, origin))?;
        file_entries.push(entry);
    }
    let mut env_entries = Vec::new();
    for (name, value) in env.prefixed(ENV_PREFIX) {
        let origin = ConfigSource::Env(name.clone());
        let entry = env_entry(&name, value?).ok_or_else(|| unsupported(name, origin))?;
        env_entries.push(entry);
    }
    for entries in [file_entries, env_entries] {
        bind_source(&mut properties, entries)?;
    }
    Ok(properties)
}

/// A higher source replaces a whole `scope` list; other properties override one by one.
fn bind_source(
    properties: &mut OAuth2ClientProperties,
    mut entries: Vec<Entry>,
) -> Result<(), ConfigError> {
    entries.sort_by_key(|entry| entry.index);
    let mut scoped: Vec<(String, bool)> = Vec::new();
    for entry in entries {
        let is_scope = entry.kind == Kind::Registration && entry.property == "scope";
        if !is_scope {
            bind(properties, entry)?;
            continue;
        }
        let registration = properties.registration.entry(entry.id.clone()).or_default();
        let indexed = entry.index.is_some();
        match scoped.iter().find(|(id, _)| *id == entry.id) {
            Some((_, was_indexed)) if *was_indexed != indexed => {
                return Err(unsupported(entry.key, entry.origin));
            }
            Some(_) => {}
            None => {
                registration.scope = Some(Vec::new());
                scoped.push((entry.id.clone(), indexed));
            }
        }
        let scopes = registration.scope.get_or_insert_default();
        if indexed {
            scopes.push(entry.value.trim().to_owned());
        } else {
            // Spring's delimited-string conversion trims each element.
            scopes.extend(
                entry
                    .value
                    .split(',')
                    .map(str::trim)
                    .filter(|scope| !scope.is_empty())
                    .map(str::to_owned),
            );
        }
    }
    Ok(())
}

fn bind(properties: &mut OAuth2ClientProperties, entry: Entry) -> Result<(), ConfigError> {
    let value = Some(entry.value);
    match entry.kind {
        Kind::Registration => {
            let registration = properties.registration.entry(entry.id).or_default();
            let RegistrationProperties {
                provider,
                client_id,
                client_secret,
                client_authentication_method,
                authorization_grant_type,
                redirect_uri,
                scope: _,
                client_name,
            } = registration;
            let target = match entry.property.as_str() {
                "provider" => provider,
                "clientid" => client_id,
                "clientsecret" => {
                    *client_secret = value.map(ClientSecret::new);
                    return Ok(());
                }
                "clientauthenticationmethod" => client_authentication_method,
                "authorizationgranttype" => authorization_grant_type,
                "redirecturi" => redirect_uri,
                "clientname" => client_name,
                _ => return Err(unsupported(entry.key, entry.origin)),
            };
            *target = value;
        }
        Kind::Provider => {
            let provider = properties.provider.entry(entry.id).or_default();
            let ProviderProperties {
                authorization_uri,
                token_uri,
                user_info_uri,
                user_info_authentication_method,
                user_name_attribute,
                jwk_set_uri,
                issuer_uri,
            } = provider;
            let target = match entry.property.as_str() {
                "authorizationuri" => authorization_uri,
                "tokenuri" => token_uri,
                "userinfouri" => user_info_uri,
                "userinfoauthenticationmethod" => user_info_authentication_method,
                "usernameattribute" => user_name_attribute,
                "jwkseturi" => jwk_set_uri,
                "issueruri" => issuer_uri,
                _ => return Err(unsupported(entry.key, entry.origin)),
            };
            *target = value;
        }
    }
    Ok(())
}

/// `registration.<id>.<property>` or `provider.<id>.<property>`, with `scope[n]` for list
/// elements. The id keeps its case, as Spring keeps map keys.
fn file_entry(key: &str, value: String, origin: ConfigSource) -> Option<Entry> {
    // The prefix may be spelled in any relaxed form; it is always four segments.
    let mut segments = key.splitn(7, '.').skip(4);
    let kind = kind(segments.next()?)?;
    let id = segments.next().filter(|id| !id.is_empty())?;
    let property = segments.next().filter(|property| !property.contains('.'))?;
    let (property, index) = match property.strip_suffix(']').and_then(|p| p.split_once('[')) {
        Some((property, index)) => (property, Some(index.parse().ok()?)),
        None => (property, None),
    };
    Some(Entry {
        kind,
        id: id.to_owned(),
        property: relaxed(property),
        index,
        value,
        key: key.to_owned(),
        origin,
    })
}

/// `<KIND>_<ID>_<PROPERTY>`, where the property is matched by its known names so that ids may
/// contain underscores; Spring lowercases ids taken from the environment. `SCOPE_<n>` is a list
/// element.
fn env_entry(name: &str, value: String) -> Option<Entry> {
    const PROPERTIES: &[&str] = &[
        "PROVIDER",
        "CLIENT_ID",
        "CLIENT_SECRET",
        "CLIENT_AUTHENTICATION_METHOD",
        "AUTHORIZATION_GRANT_TYPE",
        "REDIRECT_URI",
        "SCOPE",
        "CLIENT_NAME",
        "AUTHORIZATION_URI",
        "TOKEN_URI",
        "USER_INFO_URI",
        "USER_INFO_AUTHENTICATION_METHOD",
        "USER_NAME_ATTRIBUTE",
        "JWK_SET_URI",
        "ISSUER_URI",
    ];
    let relative = name.strip_prefix(ENV_PREFIX)?;
    let (kind_name, rest) = relative.split_once('_')?;
    let kind = kind(&kind_name.to_ascii_lowercase())?;
    let indexed_scope = rest
        .rsplit_once("_SCOPE_")
        .and_then(|(id, index)| Some((id, index.parse::<usize>().ok()?)));
    let (rest, index) = match indexed_scope {
        Some((id, index)) => (format!("{id}_SCOPE"), Some(index)),
        None => (rest.to_owned(), None),
    };
    // Spring's binding name drops the dashes (`CLIENTID`), its legacy name keeps them as
    // underscores (`CLIENT_ID`); the longest match wins so `CLIENT_ID` is not read as `ID`.
    let (id, property) = PROPERTIES
        .iter()
        .flat_map(|property| [property.to_string(), property.replace('_', "")])
        .filter_map(|property| {
            let id = rest.strip_suffix(&property)?.strip_suffix('_')?;
            Some((id.to_owned(), property))
        })
        .filter(|(id, _)| !id.is_empty())
        .min_by_key(|(id, _)| id.len())?;
    Some(Entry {
        kind,
        id: id.to_ascii_lowercase(),
        property: relaxed(&property),
        index,
        value,
        key: name.to_owned(),
        origin: ConfigSource::Env(name.to_owned()),
    })
}

fn kind(segment: &str) -> Option<Kind> {
    match segment {
        "registration" => Some(Kind::Registration),
        "provider" => Some(Kind::Provider),
        _ => None,
    }
}

fn relaxed(property: &str) -> String {
    property
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn unsupported(key: String, origin: ConfigSource) -> ConfigError {
    ConfigError::Unsupported { key, origin }
}
