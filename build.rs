fn main() {
    // Просим pkg-config найти правильные флаги для линковки SANE в Arch Linux
    if let Err(_e) = pkg_config::probe_library("sane-backends") {
        // Резервный вариант, если pkg-config не нашел описание пакета
        println!("cargo:rustc-link-lib=sane");
    }

    // Линковка libtiff для CCITT Group 4 (T.6) кодирования
    if let Err(_e) = pkg_config::probe_library("libtiff-4") {
        println!("cargo:rustc-link-lib=tiff");
    }

    // Пересборка скрипта только в случае изменения самого build.rs
    println!("cargo:rerun-if-changed=build.rs");
}
