fn main() {
    embuild::espidf::sysenv::output();

    let output = std::process::Command::new("git")
        .args(&["describe", "--tags", "--always", "--dirty"])
        .output()
        .expect("Failed to execute git command");
    let git_tag = String::from_utf8_lossy(&output.stdout).trim().to_string();
    println!("cargo:rustc-env=GIT_TAG={}", git_tag);
}
