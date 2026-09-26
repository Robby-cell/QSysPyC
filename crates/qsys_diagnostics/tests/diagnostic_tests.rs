use qsys_diagnostics::*;

#[test]
fn test_source_file_line_col_lookup() {
    let source = "line one\nline two\nline three";
    let file = SourceFile::new(FileId(1), "test.py", source);

    // Byte 0 is 'l' on Line 1, Column 1
    assert_eq!(file.location(0), (1, 1));
    // Byte 5 is ' ' on Line 1, Column 6
    assert_eq!(file.location(5), (1, 6));
    // Byte 9 is 'l' on Line 2, Column 1
    assert_eq!(file.location(9), (2, 1));
    // Byte 18 is 'l' on Line 3, Column 1
    assert_eq!(file.location(18), (3, 1));
    // Out-of-bounds byte clamps safely to EOF
    assert_eq!(file.location(999), (3, 11));
}

#[test]
fn test_source_file_slicing() {
    let source = "abcdefghij";
    let file = SourceFile::new(FileId(1), "test.py", source);

    assert_eq!(file.slice(Span::new(0, 3)), "abc");
    assert_eq!(file.slice(Span::new(3, 7)), "defg");
    // Out of bounds slicing clamps without panic
    assert_eq!(file.slice(Span::new(8, 20)), "ij");
}

#[test]
fn test_span_merging() {
    let span1 = Span::new(5, 10);
    let span2 = Span::new(15, 25);
    let merged = span1.merge(span2);

    assert_eq!(merged.start, 5);
    assert_eq!(merged.end, 25);
}

#[test]
fn test_editor_marker_conversion_for_monaco() {
    let source = "class MyPlugin:\n    bad_field = 123";
    let file = SourceFile::new(FileId(1), "plugin.py", source);

    // Create a diagnostic pointing to "bad_field" (Line 2, Col 5 to Col 14)
    let diag = Diagnostic::new(
        DiagnosticKind::syntax_error("Invalid statement in class body"),
        Span::new(20, 29),
    )
    .with_help("Declare controls using Control.<Type>()");

    let marker = diag.to_editor_marker(&file);

    assert_eq!(marker.code, "E0001");
    assert_eq!(marker.start_line_number, 2);
    assert_eq!(marker.start_column, 5);
    assert_eq!(marker.end_line_number, 2);
    assert_eq!(marker.end_column, 14);
    assert_eq!(marker.severity, Severity::Error);
    assert_eq!(
        marker.help.as_deref(),
        Some("Declare controls using Control.<Type>()")
    );
}

#[test]
fn test_diagnostics_collection_and_error_detection() {
    let mut diags = Diagnostics::new();
    assert!(!diags.has_errors());

    // Add an error
    diags.report(
        DiagnosticKind::unsupported_async("async def"),
        Span::new(0, 10),
    );
    assert!(diags.has_errors());
    assert_eq!(diags.items.len(), 1);
    assert_eq!(diags.items[0].code(), "E0101");
}

#[test]
fn test_serde_roundtrip_diagnostics() {
    let diag = Diagnostic::new(
        DiagnosticKind::UnknownControlMember {
            control_name: "Gain".to_string(),
            member: "Valu".to_string(),
            candidates: vec!["Value".to_string()],
        },
        Span::new(10, 14),
    );

    let json = serde_json::to_string(&diag).expect("Should serialize cleanly");
    let deserialized: Diagnostic = serde_json::from_str(&json).expect("Should deserialize cleanly");

    assert_eq!(deserialized.code(), "E0201");
    assert_eq!(deserialized.span, Span::new(10, 14));
    assert!(deserialized.message().contains("did you mean 'Value'?"));
}
