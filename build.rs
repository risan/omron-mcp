use std::env;
use std::fs;
use std::path::Path;

#[path = "build/render.rs"]
mod render;

const TEMPLATE: &str = "assets/home.html";
const README: &str = "README.md";

fn main() {
    for path in [TEMPLATE, README, "build/render.rs"] {
        println!("cargo:rerun-if-changed={path}");
    }

    let page = read(TEMPLATE).replace("{{content}}", &render::render(&read(README)));
    let out = Path::new(&env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("home.html");

    fs::write(out, page).expect("write home.html");
}

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"))
}
