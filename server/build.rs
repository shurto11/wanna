// web のビルド成果物を include_dir で埋め込むため、未ビルドでもディレクトリだけは用意しておく。
fn main() {
    let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/dist");
    std::fs::create_dir_all(&dist).expect("web/dist を作成できません");
    println!("cargo:rerun-if-changed=../web/dist");
}
