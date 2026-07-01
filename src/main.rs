use proc_macro2::Span;
use std::collections::HashSet;
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

    let project_root = find_project_root(&input)?;
    let crate_name = read_crate_name(&project_root)?;
    let lib_path = project_root.join("src").join("lib.rs");
    let lib_canon =
        fs::canonicalize(&lib_path).map_err(|_| "error:\nlib.rs not found".to_string())?;
    if !lib_canon.exists() {
        return Err("error:\nlib.rs not found".to_string());
    }

    let mut ctx = RenderContext::new(crate_name);
    let mut output = String::new();

    if lib_canon != input {
        output.push_str(&ctx.render_file(&lib_canon)?);
        if !output.ends_with('\n') && !output.is_empty() {
            output.push('\n');
        }
    }
    output.push_str(&ctx.render_file(&input)?);

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

fn find_project_root(input: &Path) -> Result<PathBuf> {
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

fn read_crate_name(root: &Path) -> Result<String> {
    let cargo_toml = fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|_| "error:\ncannot resolve crate".to_string())?;
    let mut section = String::new();
    let mut package_name = None;
    let mut lib_name = None;

    for raw_line in cargo_toml.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line.trim_matches(['[', ']'].as_ref()).to_string();
            continue;
        }
        if let Some(name) = parse_toml_name(line) {
            match section.as_str() {
                "package" if package_name.is_none() => package_name = Some(name),
                "lib" if lib_name.is_none() => lib_name = Some(name),
                _ => {}
            }
        }
    }

    let name = lib_name
        .or(package_name)
        .ok_or_else(|| "error:\ncannot resolve crate".to_string())?;
    Ok(normalize_crate_name(&name))
}

fn parse_toml_name(line: &str) -> Option<String> {
    let mut parts = line.splitn(2, '=');
    let key = parts.next()?.trim();
    if key != "name" {
        return None;
    }
    let value = parts.next()?.trim();
    let value = value.strip_prefix('"')?.strip_suffix('"')?;
    Some(value.to_string())
}

fn normalize_crate_name(name: &str) -> String {
    name.replace('-', "_")
}

struct RenderContext {
    crate_name: String,
    visited: HashSet<PathBuf>,
    stack: Vec<PathBuf>,
}

impl RenderContext {
    fn new(crate_name: String) -> Self {
        Self {
            crate_name,
            visited: HashSet::new(),
            stack: Vec::new(),
        }
    }

    fn render_file(&mut self, path: &Path) -> Result<String> {
        let canonical = fs::canonicalize(path).map_err(|_| module_not_found_error(path))?;
        if self.stack.contains(&canonical) {
            return Err("error:\ncyclic module detected".to_string());
        }
        if !self.visited.insert(canonical.clone()) {
            return Ok(String::new());
        }

        self.stack.push(canonical.clone());
        let result = (|| {
            let source = fs::read_to_string(path).map_err(|_| module_not_found_error(path))?;
            let file = syn::parse_file(&source)
                .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;

            self.reject_unsupported_attrs(&file.attrs)?;

            let line_starts = compute_line_starts(&source);
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
                        let child = self.resolve_module_path(path, module)?;
                        let child_text = self.render_file(&child)?;
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
                        out.push_str(&rewrite_crate_name(item_text, &self.crate_name));
                    }
                }

                cursor = end;
            }

            if cursor < source.len() {
                out.push_str(&source[cursor..]);
            }

            Ok(out)
        })();

        let _ = self.stack.pop();
        result
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

fn rewrite_crate_name(source: &str, crate_name: &str) -> String {
    let pattern = format!("{crate_name}::");
    source.replace(&pattern, "crate::")
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
