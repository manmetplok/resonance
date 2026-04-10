//! Error type for the editor runtime.

use std::fmt;

#[derive(Debug)]
pub enum EditorError {
    WaylandConnect(String),
    GlobalsMissing(&'static str),
    EglInit(String),
    EglNoConfig,
    EglContext(String),
    EglSurface(String),
    GlLoad(String),
    ThreadSpawn(std::io::Error),
    ChannelClosed,
}

impl fmt::Display for EditorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WaylandConnect(msg) => write!(f, "wayland connect failed: {msg}"),
            Self::GlobalsMissing(name) => write!(f, "required wayland global missing: {name}"),
            Self::EglInit(msg) => write!(f, "EGL initialisation failed: {msg}"),
            Self::EglNoConfig => write!(f, "no matching EGL config found"),
            Self::EglContext(msg) => write!(f, "EGL context creation failed: {msg}"),
            Self::EglSurface(msg) => write!(f, "EGL surface creation failed: {msg}"),
            Self::GlLoad(msg) => write!(f, "GL function load failed: {msg}"),
            Self::ThreadSpawn(err) => write!(f, "editor thread spawn failed: {err}"),
            Self::ChannelClosed => write!(f, "editor thread channel closed"),
        }
    }
}

impl std::error::Error for EditorError {}
