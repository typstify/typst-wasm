//! Error enum for the engine.
use std::fmt;
use std::error::Error;

/// EngineErr implements [`std::error::Error`]
#[derive(Debug)] 
pub enum EngineErr {
    PathError(String),
    UnpackError,
} 

impl fmt::Display for EngineErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PathError(path) => write!(f, "path error: {}", path),
            Self::UnpackError => write!(f, "unpack package error"),
        }
    }
}

impl Error for EngineErr {}
