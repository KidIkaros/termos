//! Every example tape must parse cleanly (errors collected, no panics).
use termos::tape::parser::parse_file;

#[test]
fn example_tapes_parse() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/examples");
    let mut parsed_any = false;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().map(|e| e == "tape").unwrap_or(false) {
            let content = std::fs::read_to_string(&path).unwrap();
            let (commands, errors) = parse_file(&content);
            assert!(
                !commands.is_empty(),
                "{} produced no commands",
                path.display()
            );
            eprintln!(
                "{}: {} commands, {} errors",
                path.display(),
                commands.len(),
                errors.len()
            );
            parsed_any = true;
        }
    }
    assert!(parsed_any, "no example tapes found");
}
