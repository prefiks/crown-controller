use std::collections::HashMap;
use std::env;
use std::fs::{File, read_to_string};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use phf_codegen::Map;

fn extract_symbols(content: &str, map: &mut HashMap<String, String>) {
    for part in content.split("#define") {
        let mut sw = part.trim_start().split_whitespace();
        match (sw.next(), sw.next()) {
            (Some(key), Some(value)) if value.starts_with("0x") => {
                let int = u32::from_str_radix(value.trim_start_matches("0x"), 16).unwrap();
                let id = if let Some(pos) = key.find('_') {
                    &key[pos + 1..]
                } else {
                    key
                };
                map.insert(id.to_lowercase(), int.to_string());
            }
            _ => {}
        }
    }
}

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dest_path = Path::new(&out_dir).join("keysyms.rs");
    let mut handle = BufWriter::new(File::create(dest_path).unwrap());

    let mut hashmap = HashMap::new();

    extract_symbols(&read_to_string("/usr/include/X11/keysymdef.h").unwrap(), &mut hashmap);
    extract_symbols(&read_to_string("/usr/include/X11/XF86keysym.h").unwrap(), &mut hashmap);

    let mut map = Map::<&str>::new();

    for (k, v) in hashmap.iter() {
        map.entry(k, v);
    }

    writeln!(&mut handle, "pub(crate) static KEYSYMS: phf::Map<&'static str, u32> = \n{};", map.build()).unwrap();
    println!("cargo:rerun-if-changed=build.rs");
}
