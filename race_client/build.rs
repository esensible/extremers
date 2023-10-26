// build.rs

use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

fn main() -> io::Result<()> {
    // Step 1: Run `npm run build` in the `client-js/` directory.
    let output = Command::new("npm")
        .args(["run", "build"])
        .current_dir("client-js")
        .output()?;
    if !output.status.success() {
        panic!("Failed to run `npm run build`");
    }

    // Determine the number of files in `client-js/dist/`
    let file_count = fs::read_dir("client-js/dist")?
        .filter(|entry| entry.as_ref().map(|e| e.path().is_file()).unwrap_or(false))
        .count();

    // Step 2: Read the files from `client-js/dist/` and create STATIC_FILES array.
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("static_files.rs");
    let mut f = File::create(dest_path)?;

    write!(
        &mut f,
        "static STATIC_FILES: [(&str, &[u8]); {}] = [",
        file_count
    )?;

    for entry in fs::read_dir("client-js/dist")? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let filename = path.file_name().unwrap().to_string_lossy();
            let data = fs::read(&path)?;
            let data_elements = data
                .iter()
                .map(|byte| format!("{}", byte))
                .collect::<Vec<_>>()
                .join(", ");
            write!(
                &mut f,
                "    (\"{}\", &[\n        {}\n    ]),\n",
                filename, data_elements
            )?;
        }
    }

    writeln!(&mut f, "];")?;
    Ok(())
}
