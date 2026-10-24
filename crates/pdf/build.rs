use std::{
    env, fs,
    path::{Component, Path, PathBuf},
};

const MAX_FONTS: usize = 128;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

fn manifest_fonts(root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let manifest = root.join("fonts/manifest.txt");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let contents = fs::read_to_string(&manifest).map_err(|error| format!("{}: {error}", manifest.display()))?;
    let canonical_root = root.canonicalize().map_err(|error| format!("{}: {error}", root.display()))?;
    let mut fonts = Vec::new();
    let mut total_bytes = 0u64;

    for (line_number, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(" | ").map(str::trim).collect();
        let Some(file) = fields.get(2).copied().filter(|_| fields.len() == 8) else {
            return Err(format!("{}:{}: expected eight manifest fields separated by a pipe delimiter", manifest.display(), line_number + 1));
        };
        let relative = Path::new(file);
        if relative.is_absolute()
            || relative.components().any(|part| matches!(part, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
        {
            return Err(format!("{}:{}: font path must stay inside the checkout: {file}", manifest.display(), line_number + 1));
        }
        let path = root.join(relative).canonicalize().map_err(|error| format!("{file}: {error}"))?;
        if !path.starts_with(&canonical_root) {
            return Err(format!("{}:{}: font path resolves outside the checkout: {file}", manifest.display(), line_number + 1));
        }
        let size = fs::metadata(&path).map_err(|error| format!("{file}: {error}"))?.len();
        total_bytes = total_bytes.checked_add(size).ok_or_else(|| "craft-fonts total size overflow".to_string())?;
        if total_bytes > MAX_TOTAL_BYTES {
            return Err(format!("fonts in the manifest exceed the {MAX_TOTAL_BYTES}-byte build limit"));
        }
        fonts.push((file.to_string(), path));
        if fonts.len() > MAX_FONTS {
            return Err(format!("fonts/manifest.txt contains more than {MAX_FONTS} font files"));
        }
    }
    Ok(fonts)
}

fn generated_catalog(root: &Path) -> Result<String, String> {
    let mut generated = String::from("pub const CRAFT_FONTS: &[(&str, &[u8])] = &[\n");
    for (relative, path) in manifest_fonts(root)? {
        println!("cargo:rerun-if-changed={}", path.display());
        let absolute = path.to_string_lossy();
        generated.push_str(&format!("({relative:?}, include_bytes!({absolute:?})),\n"));
    }
    generated.push_str("];\n");
    Ok(generated)
}

fn main() -> Result<(), String> {
    println!("cargo:rerun-if-env-changed=CRAFT_FONTS_DIR");
    println!("cargo:rerun-if-env-changed=CRAFT_FONTS_REQUIRED");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_ARCH");

    // The web UI owns its font budget. PDF faces are supplied per export through `pdfFonts`,
    // so this crate must not add the whole desktop font manifest to the WASM binary.
    let wasm = env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|arch| arch == "wasm32");
    let required = !wasm && env::var_os("CRAFT_FONTS_REQUIRED").is_some();
    let mut generated = String::from("pub const CRAFT_FONTS: &[(&str, &[u8])] = &[];\n");
    let configured = if wasm { None } else { env::var_os("CRAFT_FONTS_DIR").map(PathBuf::from) };
    match configured {
        Some(configured) => match configured.canonicalize() {
            Ok(root) => match generated_catalog(&root) {
                Ok(catalog) => generated = catalog,
                Err(error) if required => return Err(format!("CRAFT_FONTS_DIR={}: {error}", root.display())),
                Err(error) => println!("cargo:warning=building without craft-fonts: {error}"),
            },
            Err(error) if required => return Err(format!("CRAFT_FONTS_DIR={}: {error}", configured.display())),
            Err(error) => println!("cargo:warning=building without craft-fonts: CRAFT_FONTS_DIR={}: {error}", configured.display()),
        },
        None if required => return Err("CRAFT_FONTS_REQUIRED is set but CRAFT_FONTS_DIR is not".to_string()),
        None => {}
    }

    let out_dir = env::var_os("OUT_DIR").ok_or_else(|| "OUT_DIR is missing".to_string())?;
    let out = PathBuf::from(out_dir).join("craft_fonts.rs");
    fs::write(&out, generated).map_err(|error| format!("{}: {error}", out.display()))?;
    Ok(())
}
