use std::path::PathBuf;

use secrecy::{ExposeSecret, SecretString};

use crate::config::helpers::{optional_env, parse_optional_env};
use crate::error::ConfigError;

/// Which database backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DatabaseBackend {
    /// PostgreSQL via deadpool-postgres (default).
    #[default]
    Postgres,
    /// libSQL/Turso embedded database.
    LibSql,
}

impl std::fmt::Display for DatabaseBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Postgres => write!(f, "postgres"),
            Self::LibSql => write!(f, "libsql"),
        }
    }
}

impl std::str::FromStr for DatabaseBackend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "postgres" | "postgresql" | "pg" => Ok(Self::Postgres),
            "libsql" | "turso" | "sqlite" => Ok(Self::LibSql),
            _ => Err(format!(
                "invalid database backend '{}', expected 'postgres' or 'libsql'",
                s
            )),
        }
    }
}

/// Database configuration.
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// Which backend to use (default: Postgres).
    pub backend: DatabaseBackend,

    // -- PostgreSQL fields --
    pub url: SecretString,
    pub pool_size: usize,

    // -- libSQL fields --
    /// Path to local libSQL database file (default: ~/.clawyer/clawyer.db).
    pub libsql_path: Option<PathBuf>,
    /// Turso cloud URL for remote sync (optional).
    pub libsql_url: Option<String>,
    /// Turso auth token (required when libsql_url is set).
    pub libsql_auth_token: Option<SecretString>,
}

impl DatabaseConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Self::resolve_internal(false)
    }

    /// Resolve database config for early startup.
    ///
    /// When `skip_db_validation` is true (used with `--no-db`), missing
    /// `DATABASE_URL` is tolerated because no DB connection will be created.
    pub(crate) fn resolve_for_startup(skip_db_validation: bool) -> Result<Self, ConfigError> {
        Self::resolve_internal(skip_db_validation)
    }

    fn resolve_internal(skip_db_validation: bool) -> Result<Self, ConfigError> {
        let backend: DatabaseBackend = if let Some(b) = optional_env("DATABASE_BACKEND")? {
            b.parse().map_err(|e| ConfigError::InvalidValue {
                key: "DATABASE_BACKEND".to_string(),
                message: e,
            })?
        } else {
            DatabaseBackend::default()
        };

        // PostgreSQL URL is required only when using the postgres backend.
        // For libsql backend, default to an empty placeholder.
        // DATABASE_URL is loaded from ~/.clawyer/.env via dotenvy early in startup.
        let url = optional_env("DATABASE_URL")?
            .or_else(|| {
                if backend == DatabaseBackend::LibSql || skip_db_validation {
                    Some("unused://libsql".to_string())
                } else {
                    None
                }
            })
            .ok_or_else(|| ConfigError::MissingRequired {
                key: "DATABASE_URL".to_string(),
                hint: "Run 'clawyer onboard' or set DATABASE_URL environment variable".to_string(),
            })?;

        let pool_size = parse_optional_env("DATABASE_POOL_SIZE", 10)?;

        let libsql_path = optional_env("LIBSQL_PATH")?.map(PathBuf::from).or_else(|| {
            if backend == DatabaseBackend::LibSql {
                Some(default_libsql_path())
            } else {
                None
            }
        });

        let libsql_url = optional_env("LIBSQL_URL")?;
        let libsql_auth_token = optional_env("LIBSQL_AUTH_TOKEN")?.map(SecretString::from);

        if libsql_url.is_some() && libsql_auth_token.is_none() {
            return Err(ConfigError::MissingRequired {
                key: "LIBSQL_AUTH_TOKEN".to_string(),
                hint: "LIBSQL_AUTH_TOKEN is required when LIBSQL_URL is set".to_string(),
            });
        }

        Ok(Self {
            backend,
            url: SecretString::from(url),
            pool_size,
            libsql_path,
            libsql_url,
            libsql_auth_token,
        })
    }

    /// Get the database URL (exposes the secret).
    pub fn url(&self) -> &str {
        self.url.expose_secret()
    }
}

/// Default libSQL database path (~/.clawyer/clawyer.db).
pub fn default_libsql_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".clawyer")
        .join("clawyer.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::ENV_MUTEX;

    struct EnvGuard {
        key: &'static str,
        value: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: guarded by ENV_MUTEX in tests.
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                key,
                value: previous,
            }
        }

        fn clear(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: guarded by ENV_MUTEX in tests.
            unsafe {
                std::env::remove_var(key);
            }
            Self {
                key,
                value: previous,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: guarded by ENV_MUTEX in tests.
            unsafe {
                if let Some(ref val) = self.value {
                    std::env::set_var(self.key, val);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    #[test]
    fn resolve_strict_requires_database_url_for_postgres() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        let _db_url = EnvGuard::clear("DATABASE_URL");
        let _backend = EnvGuard::set("DATABASE_BACKEND", "postgres");

        let err = DatabaseConfig::resolve().expect_err("strict resolve should fail");
        match err {
            ConfigError::MissingRequired { key, .. } => assert_eq!(key, "DATABASE_URL"),
            other => panic!("expected MissingRequired(DATABASE_URL), got {other:?}"),
        }
    }

    #[test]
    fn resolve_startup_allows_missing_database_url_when_skipping_validation() {
        let _guard = ENV_MUTEX.lock().expect("env mutex poisoned");
        let _db_url = EnvGuard::clear("DATABASE_URL");
        let _backend = EnvGuard::set("DATABASE_BACKEND", "postgres");

        let cfg = DatabaseConfig::resolve_for_startup(true)
            .expect("startup resolve should tolerate missing DATABASE_URL");
        assert_eq!(cfg.backend, DatabaseBackend::Postgres);
        assert_eq!(cfg.url(), "unused://libsql");
    }
}
