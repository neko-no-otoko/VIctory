//! Offline DXIL conversion through the user's Apple Metal Shader Converter.

use std::path::Path;

use anyhow::{Context, Result};
use clap::ValueEnum;
use victory_metal::converter;

#[derive(Clone, Copy, ValueEnum)]
pub enum Stage {
    Vertex,
    Fragment,
    Mesh,
    Compute,
}

pub fn convert(input: &Path, output: &Path, library: &Path, stage: Stage) -> Result<()> {
    victory_core::ensure_outside_repo(output)?;
    let bytes = std::fs::read(input).with_context(|| format!("reading {}", input.display()))?;
    let stage = match stage {
        Stage::Vertex => converter::Stage::Vertex,
        Stage::Fragment => converter::Stage::Fragment,
        Stage::Mesh => converter::Stage::Mesh,
        Stage::Compute => converter::Stage::Compute,
    };
    let data = converter::convert(library, &bytes, stage).map_err(anyhow::Error::msg)?;
    std::fs::write(output, data).with_context(|| format!("writing {}", output.display()))?;
    println!("wrote {}", output.display());
    Ok(())
}
