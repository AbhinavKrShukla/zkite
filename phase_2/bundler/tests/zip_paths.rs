use zkite_bundler::zip_extras::validate_entry_path;
use std::collections::HashSet;

#[test]
fn rejects_windows_escape_paths_on_every_host() {
    for path in ["C:/outside.so", "C:outside.so", r"..\outside.so", r"\\server\share\outside.so", "assets/\0invalid"] {
        let result = std::panic::catch_unwind(|| {
            validate_entry_path(path, &mut HashSet::new());
        });
        assert!(result.is_err(), "accepted unsafe archive path {path:?}");
    }
    validate_entry_path("assets/package/module.py", &mut HashSet::new());
}
