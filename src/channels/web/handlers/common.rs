//! Shared web handler helper facade.
//!
//! This module intentionally re-exports helper modules so existing call sites
//! can keep using `server::...` helper names during decomposition.

pub(crate) use crate::channels::web::handlers::helpers::legal::*;
pub(crate) use crate::channels::web::handlers::helpers::mappers::*;
pub(crate) use crate::channels::web::handlers::helpers::matter::*;
pub(crate) use crate::channels::web::handlers::helpers::parsing::*;
