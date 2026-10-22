use std::{
    env, fs,
    path::{Path, PathBuf},
};

const MAX_FONTS: usize = 128;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

fn collect(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files)?;
        } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc"))
        {
            files.push(path);
            if files.len() > MAX_FONTS {
                return Err(std::io::Error::other("CRAFT_FONTS_DIR contains more than 128 font files"));
            }
        }
    }
    Ok(())
}

fn main() {
    println!("cargo:rerun-if-env-changed=CRAFT_FONTS_DIR");
    let mut files = Vec::new();
    if let Some(root) = env::var_os("CRAFT_FONTS_DIR") {
        let root = PathBuf::from(root);
        if root.is_dir() {
            if let Err(error) = collect(&root, &mut files) {
                println!("cargo:warning=Could not scan CRAFT_FONTS_DIR: {error}");
            }
        } else {
            println!("cargo:warning=CRAFT_FONTS_DIR is set but is not a directory");
        }
    }
    files.sort();
    let mut total = 0u64;
    let mut valid = Vec::new();
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        match fs::metadata(&path) {
            Ok(meta) if meta.len() <= MAX_TOTAL_BYTES.saturating_sub(total) => {
                total += meta.len();
                valid.push(path);
            }
            Ok(_) => println!("cargo:warning=Skipping font over the 64 MiB CRAFT_FONTS_DIR total limit: {}", path.display()),
            Err(error) => println!("cargo:warning=Skipping unreadable craft font {}: {error}", path.display()),
        }
    }
    let mut generated = String::from("pub const CRAFT_FONTS: &[(&str, &[u8])] = &[\n");
    for path in valid {
        let path_literal = format!("{}", path.display());
        generated.push_str(&format!("({path_literal:?}, include_bytes!({path_literal:?})),\n"));
    }
    generated.push_str("];");
    let Some(out_dir) = env::var_os("OUT_DIR") else {
        println!("cargo:warning=OUT_DIR is missing; optional craft font catalog was not generated");
        return;
    };
    let out = PathBuf::from(out_dir).join("craft_fonts.rs");
    if let Err(error) = fs::write(out, generated) {
        println!("cargo:warning=Could not write generated craft font catalog: {error}");
    }
}
