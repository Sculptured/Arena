//! `arena-package.toml` and the registry of installed packages.
use anyhow::{anyhow, bail, Context, Result};
use arena_part::proto::{valid_package_name, PROTOCOL};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Integration,
    Benchmark,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Run {
    Command(Vec<String>),
    Image(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Needs {
    pub gpu_vram_gb: u32,
    pub services: Vec<String>,
    pub network: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Manifest {
    pub kind: Kind,
    pub name: String,
    pub version: String,
    pub protocol: u32,
    pub run: Run,
    pub needs: Needs,
    /// The folder holding the manifest.
    pub dir: PathBuf,
    /// The `[[example]]` entries, `{tool, args}`.
    pub examples: Vec<Value>,
    pub config_schema: Value,
}

fn str_of(v: &Value, key: &str) -> Result<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("`{key}` is missing or not a string"))
}

impl Manifest {
    /// Reads `<dir>/arena-package.toml`.
    pub fn load(dir: &Path) -> Result<Manifest> {
        let file = dir.join("arena-package.toml");
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        Self::parse(&text, dir).with_context(|| format!("in {}", file.display()))
    }

    fn parse(text: &str, dir: &Path) -> Result<Manifest> {
        let v: Value = serde_json::to_value(text.parse::<toml::Table>()?)?;
        let kind = parse_kind(&v)?;
        let name = str_of(&v, "name")?;
        if !valid_package_name(&name) {
            bail!("package name {name:?} must match [a-z0-9-]{{1,40}}");
        }
        let protocol = v.get("protocol").and_then(Value::as_u64).unwrap_or(0) as u32;
        if protocol != PROTOCOL {
            bail!("protocol {protocol} is not supported, only {PROTOCOL}");
        }
        let run = parse_run(v.get("run").ok_or_else(|| anyhow!("`run` is missing"))?)?;
        Ok(Manifest {
            kind,
            name,
            version: str_of(&v, "version")?,
            protocol,
            run,
            needs: parse_needs(v.get("needs"))?,
            dir: dir.to_path_buf(),
            examples: v
                .get("example")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            config_schema: v.get("config_schema").cloned().unwrap_or(Value::Null),
        })
    }
}

fn parse_kind(v: &Value) -> Result<Kind> {
    Ok(match str_of(v, "kind")?.as_str() {
        "integration" => Kind::Integration,
        "benchmark" => Kind::Benchmark,
        k => bail!("kind must be integration or benchmark, not {k:?}"),
    })
}

fn parse_run(run: &Value) -> Result<Run> {
    Ok(match (run.get("command"), run.get("image")) {
        (Some(c), None) => {
            let c: Vec<String> = serde_json::from_value(c.clone()).context("run.command")?;
            if c.is_empty() {
                bail!("run.command is empty");
            }
            Run::Command(c)
        }
        (None, Some(i)) => Run::Image(
            i.as_str()
                .ok_or_else(|| anyhow!("run.image must be a string"))?
                .into(),
        ),
        _ => bail!("run needs exactly one of command or image"),
    })
}

fn parse_needs(needs: Option<&Value>) -> Result<Needs> {
    let Some(n) = needs else {
        return Ok(Needs::default());
    };
    Ok(Needs {
        gpu_vram_gb: n.get("gpu_vram_gb").and_then(Value::as_u64).unwrap_or(0) as u32,
        services: match n.get("services") {
            Some(s) => serde_json::from_value(s.clone()).context("needs.services")?,
            None => vec![],
        },
        network: n.get("network").and_then(Value::as_str).map(str::to_string),
    })
}

/// The installed packages, by name.
#[derive(Debug, Default)]
pub struct Registry {
    by_name: BTreeMap<String, Manifest>,
}

impl Registry {
    /// Loads every `<root>/<folder>/arena-package.toml`. A missing root is skipped; a duplicate name is an error.
    pub fn scan(roots: &[PathBuf]) -> Result<Registry> {
        let mut by_name = BTreeMap::new();
        for root in roots {
            let Ok(rd) = std::fs::read_dir(root) else {
                continue;
            };
            let mut dirs: Vec<PathBuf> = rd.filter_map(|e| Some(e.ok()?.path())).collect();
            dirs.sort();
            for d in dirs
                .into_iter()
                .filter(|d| d.join("arena-package.toml").is_file())
            {
                let m = Manifest::load(&d)?;
                if let Some(old) = by_name.get(&m.name) {
                    let old: &Manifest = old;
                    bail!(
                        "package {:?} is installed twice: {} and {}",
                        m.name,
                        old.dir.display(),
                        d.display()
                    );
                }
                by_name.insert(m.name.clone(), m);
            }
        }
        Ok(Registry { by_name })
    }

    /// A registry of the one package in `dir`.
    pub fn from_dir(dir: &Path) -> Result<Registry> {
        let m = Manifest::load(dir)?;
        let mut by_name = BTreeMap::new();
        by_name.insert(m.name.clone(), m);
        Ok(Registry { by_name })
    }

    pub fn get(&self, name: &str) -> Option<&Manifest> {
        self.by_name.get(name)
    }

    fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Manifest> {
        self.by_name.values().filter(move |m| m.kind == kind)
    }

    pub fn benchmarks(&self) -> impl Iterator<Item = &Manifest> {
        self.of_kind(Kind::Benchmark)
    }

    pub fn integrations(&self) -> impl Iterator<Item = &Manifest> {
        self.of_kind(Kind::Integration)
    }
}
