use proc_macro2::Span;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use syn::{spanned::Spanned, Attribute, Item};

type Result<T> = std::result::Result<T, String>;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = parse_args()?;
    let input = fs::canonicalize(&args.input)
        .map_err(|_| format!("error:\ninput file not found: {}", args.input.display()))?;

    let workspace_root = find_workspace_root(&input)?;
    let workspace = Workspace::load(&workspace_root)?;
    let mut bundler = Bundler::new(workspace);

    let output = bundler.bundle_input(&input)?;

    match args.output {
        OutputTarget::Stdout => {
            print!("{output}");
        }
        OutputTarget::File(path) => {
            fs::write(&path, output)
                .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
        }
    }

    Ok(())
}

struct Args {
    input: PathBuf,
    output: OutputTarget,
}

enum OutputTarget {
    Stdout,
    File(PathBuf),
}

fn parse_args() -> Result<Args> {
    let mut input = None;
    let mut output = OutputTarget::Stdout;
    let mut saw_o = false;
    let mut saw_stdout = false;

    let mut iter = env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-o" => {
                let path = iter
                    .next()
                    .ok_or_else(|| "missing output path after -o".to_string())?;
                saw_o = true;
                if saw_stdout {
                    return Err("cannot specify both -o and --stdout".to_string());
                }
                output = OutputTarget::File(PathBuf::from(path));
            }
            "--stdout" => {
                saw_stdout = true;
                if saw_o {
                    return Err("cannot specify both -o and --stdout".to_string());
                }
                output = OutputTarget::Stdout;
            }
            _ if arg.starts_with('-') => {
                return Err(format!("unknown option: {arg}"));
            }
            _ => {
                if input.is_some() {
                    return Err("multiple input files are not supported".to_string());
                }
                input = Some(PathBuf::from(arg));
            }
        }
    }

    let input = input.ok_or_else(|| "missing input file".to_string())?;
    Ok(Args { input, output })
}

fn find_workspace_root(input: &Path) -> Result<PathBuf> {
    let mut current = input
        .parent()
        .ok_or_else(|| "error:\ncannot resolve crate".to_string())?;
    loop {
        if current.join("Cargo.toml").exists() {
            return Ok(current.to_path_buf());
        }
        current = current
            .parent()
            .ok_or_else(|| "error:\ncannot resolve crate".to_string())?;
    }
}

struct Workspace {
    manifest: Manifest,
}

impl Workspace {
    fn load(root: &Path) -> Result<Self> {
        let manifest = Manifest::load(&root.join("Cargo.toml"))?;
        Ok(Self { manifest })
    }
}

#[derive(Clone)]
struct Manifest {
    manifest_dir: PathBuf,
    package_name: Option<String>,
    lib_name: Option<String>,
    lib_path: Option<PathBuf>,
    path_deps: HashMap<String, PathBuf>,
}

impl Manifest {
    fn load(manifest_path: &Path) -> Result<Self> {
        let manifest_dir = manifest_path
            .parent()
            .ok_or_else(|| "error:\ncannot resolve crate".to_string())?
            .to_path_buf();
        let text = fs::read_to_string(manifest_path)
            .map_err(|_| "error:\ncannot resolve crate".to_string())?;

        let mut package_name = None;
        let mut lib_name = None;
        let mut lib_path = None;
        let mut path_deps = HashMap::new();
        let mut section = String::new();
        let mut section_dep_alias = None::<String>;

        for raw_line in text.lines() {
            let line = raw_line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line.trim_matches(['[', ']'].as_ref()).to_string();
                section_dep_alias = dependency_section_alias(&section);
                continue;
            }

            if section == "package" {
                if let Some(name) = parse_toml_string_field(line, "name") {
                    package_name = Some(name);
                }
                continue;
            }

            if section == "lib" {
                if let Some(name) = parse_toml_string_field(line, "name") {
                    lib_name = Some(name);
                }
                if let Some(path) = parse_toml_string_field(line, "path") {
                    lib_path = Some(PathBuf::from(path));
                }
                continue;
            }

            if let Some(alias) = &section_dep_alias {
                if let Some(path) = parse_toml_string_field(line, "path") {
                    path_deps.insert(normalize_crate_name(alias), manifest_dir.join(path));
                }
                continue;
            }

            if is_dependency_section(&section) {
                if let Some((alias, path)) = parse_dependency_line(line) {
                    path_deps.insert(normalize_crate_name(&alias), manifest_dir.join(path));
                }
            }
        }

        Ok(Self {
            manifest_dir,
            package_name,
            lib_name,
            lib_path,
            path_deps,
        })
    }

    fn self_alias(&self) -> Option<String> {
        self.lib_name
            .as_ref()
            .or(self.package_name.as_ref())
            .map(|name| normalize_crate_name(name))
    }

    fn local_aliases(&self) -> Vec<String> {
        let mut aliases = Vec::new();
        if let Some(alias) = self.self_alias() {
            aliases.push(alias);
        }
        aliases.extend(self.path_deps.keys().cloned());
        aliases.sort();
        aliases.dedup();
        aliases
    }

    fn dep_aliases(&self) -> Vec<String> {
        let mut aliases: Vec<_> = self.path_deps.keys().cloned().collect();
        aliases.sort();
        aliases.dedup();
        aliases
    }

    fn lib_path(&self) -> PathBuf {
        self.manifest_dir.join(
            self.lib_path
                .clone()
                .unwrap_or_else(|| PathBuf::from("src/lib.rs")),
        )
    }

    fn resolve_alias(&self, alias: &str) -> Option<PathBuf> {
        let alias = normalize_crate_name(alias);
        if self.self_alias().as_deref() == Some(alias.as_str()) {
            return Some(self.lib_path());
        }
        self.path_deps.get(&alias).map(|dir| {
            let manifest = dir.join("Cargo.toml");
            let dep_manifest = Manifest::load(&manifest).ok();
            dep_manifest
                .map(|m| m.lib_path())
                .unwrap_or_else(|| dir.join("src/lib.rs"))
        })
    }
}

fn is_dependency_section(section: &str) -> bool {
    section == "dependencies"
        || section == "dev-dependencies"
        || section == "build-dependencies"
        || section.starts_with("dependencies.")
        || section.starts_with("dev-dependencies.")
        || section.starts_with("build-dependencies.")
        || section == "workspace.dependencies"
}

fn dependency_section_alias(section: &str) -> Option<String> {
    for prefix in [
        "dependencies.",
        "dev-dependencies.",
        "build-dependencies.",
        "workspace.dependencies.",
    ] {
        if let Some(alias) = section.strip_prefix(prefix) {
            if !alias.is_empty() {
                return Some(alias.to_string());
            }
        }
    }
    None
}

fn parse_toml_string_field(line: &str, key: &str) -> Option<String> {
    let mut parts = line.splitn(2, '=');
    let found_key = parts.next()?.trim();
    if found_key != key {
        return None;
    }
    let value = parts.next()?.trim();
    let value = value.strip_prefix('"')?.strip_suffix('"')?;
    Some(value.to_string())
}

fn parse_dependency_line(line: &str) -> Option<(String, String)> {
    let mut parts = line.splitn(2, '=');
    let alias = parts.next()?.trim();
    let rhs = parts.next()?.trim();
    if !rhs.starts_with('{') || !rhs.ends_with('}') {
        return None;
    }
    let inner = &rhs[1..rhs.len() - 1];
    for segment in inner.split(',') {
        let segment = segment.trim();
        if let Some(path) = parse_toml_string_field(segment, "path") {
            return Some((alias.to_string(), path));
        }
    }
    None
}

fn normalize_crate_name(name: &str) -> String {
    name.replace('-', "_")
}

struct Bundler {
    workspace: Workspace,
    manifest_cache: HashMap<PathBuf, Manifest>,
    visited_crates: HashSet<PathBuf>,
}

impl Bundler {
    fn new(workspace: Workspace) -> Self {
        let mut manifest_cache = HashMap::new();
        manifest_cache.insert(
            workspace.manifest.manifest_dir.clone(),
            workspace.manifest.clone(),
        );
        Self {
            workspace,
            manifest_cache,
            visited_crates: HashSet::new(),
        }
    }

    fn bundle_input(&mut self, input: &Path) -> Result<String> {
        let manifest = self.workspace.manifest.clone();
        let source = fs::read_to_string(input)
            .map_err(|_| format!("error:\ninput file not found: {}", input.display()))?;
        let file = syn::parse_file(&source)
            .map_err(|e| format!("failed to parse {}: {e}", input.display()))?;

        self.render_source(
            &source,
            &file,
            input,
            &manifest,
            &manifest.local_aliases(),
            &manifest.local_aliases(),
        )
    }

    fn render_crate(&mut self, lib_path: &Path) -> Result<String> {
        let canonical = fs::canonicalize(lib_path).map_err(|_| module_not_found_error(lib_path))?;
        if !self.visited_crates.insert(canonical.clone()) {
            return Ok(String::new());
        }

        let manifest_dir = lib_path
            .parent()
            .and_then(|p| p.parent())
            .ok_or_else(|| module_not_found_error(lib_path))?
            .to_path_buf();
        let manifest = self.manifest(&manifest_dir)?;
        let source = fs::read_to_string(lib_path).map_err(|_| module_not_found_error(lib_path))?;
        let file = syn::parse_file(&source)
            .map_err(|e| format!("failed to parse {}: {e}", lib_path.display()))?;

        self.render_source(
            &source,
            &file,
            lib_path,
            &manifest,
            &manifest.local_aliases(),
            &manifest.dep_aliases(),
        )
    }

    fn render_source(
        &mut self,
        source: &str,
        file: &syn::File,
        source_path: &Path,
        manifest: &Manifest,
        aliases_to_rewrite: &[String],
        child_aliases_to_render: &[String],
    ) -> Result<String> {
        self.reject_unsupported_attrs(&file.attrs)?;

        let used_aliases = self.used_local_aliases(source, child_aliases_to_render);
        let mut output = String::new();
        for alias in used_aliases {
            if let Some(lib_path) = manifest.resolve_alias(&alias) {
                if lib_path.exists() {
                    output.push_str(&self.render_crate(&lib_path)?);
                    if !output.ends_with('\n') && !output.is_empty() {
                        output.push('\n');
                    }
                }
            }
        }

        let line_starts = compute_line_starts(source);
        let mut out = String::new();
        let mut cursor = 0usize;

        for item in &file.items {
            self.reject_unsupported_attrs(item_attrs(item))?;
            let span = item.span();
            let (start, end) = span_to_range(span, &line_starts, source.len())?;
            if start > cursor {
                out.push_str(&source[cursor..start]);
            }
            let item_text = &source[start..end];

            match item {
                Item::Mod(module) if module.content.is_none() => {
                    let child = self.resolve_module_path(source_path, module)?;
                    let child_source =
                        fs::read_to_string(&child).map_err(|_| module_not_found_error(&child))?;
                    let child_file = syn::parse_file(&child_source)
                        .map_err(|e| format!("failed to parse {}: {e}", child.display()))?;
                    let child_text = self.render_source(
                        &child_source,
                        &child_file,
                        &child,
                        manifest,
                        aliases_to_rewrite,
                        child_aliases_to_render,
                    )?;
                    if !child_text.is_empty() {
                        out.push_str(&inline_module_header(item_text));
                        out.push('\n');
                        out.push_str(&child_text);
                        if !child_text.ends_with('\n') {
                            out.push('\n');
                        }
                        out.push('}');
                    }
                }
                _ => {
                    out.push_str(&rewrite_local_crate_refs(item_text, aliases_to_rewrite));
                }
            }

            cursor = end;
        }

        if cursor < source.len() {
            out.push_str(&source[cursor..]);
        }

        let _ = manifest;
        output.push_str(&out);
        Ok(output)
    }

    fn manifest(&mut self, manifest_dir: &Path) -> Result<Manifest> {
        if let Some(manifest) = self.manifest_cache.get(manifest_dir) {
            return Ok(manifest.clone());
        }
        let manifest = Manifest::load(&manifest_dir.join("Cargo.toml"))?;
        self.manifest_cache
            .insert(manifest_dir.to_path_buf(), manifest.clone());
        Ok(manifest)
    }

    fn used_local_aliases(&self, source: &str, candidates: &[String]) -> Vec<String> {
        let mut aliases: Vec<String> = candidates
            .iter()
            .filter(|alias| source.contains(&format!("{alias}::")))
            .cloned()
            .collect();
        aliases.sort_by_key(|alias| std::cmp::Reverse(alias.len()));
        aliases.dedup();
        aliases
    }

    fn reject_unsupported_attrs(&self, attrs: &[Attribute]) -> Result<()> {
        for attr in attrs {
            if is_unsupported_attr(attr) {
                return Err(format!(
                    "error:\nunsupported attribute: {}",
                    attr.path().segments[0].ident
                ));
            }
        }
        Ok(())
    }

    fn resolve_module_path(&self, current_file: &Path, module: &syn::ItemMod) -> Result<PathBuf> {
        let module_name = module.ident.to_string();
        let dir = current_file
            .parent()
            .ok_or_else(|| format!("error:\nmodule \"{module_name}\" not found"))?;

        let candidate_rs = dir.join(format!("{module_name}.rs"));
        if candidate_rs.exists() {
            return Ok(candidate_rs);
        }
        let candidate_mod = dir.join(&module_name).join("mod.rs");
        if candidate_mod.exists() {
            return Ok(candidate_mod);
        }

        Err(format!("error:\nmodule \"{module_name}\" not found"))
    }
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        Item::Verbatim(_) => &[],
        _ => &[],
    }
}

fn is_unsupported_attr(attr: &Attribute) -> bool {
    matches!(
        attr.path()
            .segments
            .first()
            .map(|seg| seg.ident.to_string())
            .as_deref(),
        Some("cfg" | "cfg_attr" | "path")
    )
}

fn rewrite_local_crate_refs(source: &str, aliases: &[String]) -> String {
    let mut output = source.to_string();
    for alias in aliases {
        let pattern = format!("{alias}::");
        output = output.replace(&pattern, "crate::");
    }
    output
}

fn inline_module_header(item_text: &str) -> String {
    let mut header = item_text.to_string();
    if let Some(pos) = header.rfind(';') {
        header.replace_range(pos..=pos, " {");
    } else {
        header.push_str(" {");
    }
    header
}

fn compute_line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (idx, ch) in source.char_indices() {
        if ch == '\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

fn span_to_range(span: Span, line_starts: &[usize], max: usize) -> Result<(usize, usize)> {
    let start = loc_to_offset(span.start().line, span.start().column, line_starts)?;
    let end = loc_to_offset(span.end().line, span.end().column, line_starts)?;
    Ok((start.min(max), end.min(max)))
}

fn loc_to_offset(line: usize, column: usize, line_starts: &[usize]) -> Result<usize> {
    let line_index = line
        .checked_sub(1)
        .ok_or_else(|| "invalid span line".to_string())?;
    let base = *line_starts
        .get(line_index)
        .ok_or_else(|| "invalid span line".to_string())?;
    Ok(base + column)
}

fn module_not_found_error(path: &Path) -> String {
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
    format!("error:\nmodule \"{name}\" not found")
}
