use super::*;

// --- compile_query: the final-token prefix contract -------------------------

#[test]
fn plain_final_word_is_prefix_matched() {
    assert_eq!(
        compile_query("quarterly quart"),
        CompiledQuery::HeadAndPrefix("quarterly".to_string(), "quart:*".to_string())
    );
}

#[test]
fn single_token_becomes_a_bare_prefix_query() {
    assert_eq!(
        compile_query("quart"),
        CompiledQuery::Prefix("quart:*".to_string())
    );
}

#[test]
fn trailing_whitespace_opts_out_of_prefixing() {
    assert_eq!(
        compile_query("quart "),
        CompiledQuery::Plain("quart".to_string())
    );
}

#[test]
fn negated_final_token_is_never_prefixed() {
    assert_eq!(
        compile_query("review -draft"),
        CompiledQuery::Plain("review -draft".to_string())
    );
}

#[test]
fn quoted_final_token_is_never_prefixed() {
    assert_eq!(
        compile_query(r#"prep "quarterly review""#),
        CompiledQuery::Plain(r#"prep "quarterly review""#.to_string())
    );
}

#[test]
fn unbalanced_quote_disables_prefixing() {
    // The last token sits inside a still-open phrase.
    assert_eq!(
        compile_query(r#""quarterly rev"#),
        CompiledQuery::Plain(r#""quarterly rev"#.to_string())
    );
}

#[test]
fn prefix_lexeme_is_sanitized_to_alphanumerics() {
    assert_eq!(
        compile_query("review q3!"),
        CompiledQuery::HeadAndPrefix("review".to_string(), "q3:*".to_string())
    );
}

#[test]
fn symbol_only_final_token_falls_back_to_plain() {
    assert_eq!(
        compile_query("review !!"),
        CompiledQuery::Plain("review !!".to_string())
    );
}

// --- Cursor: bit-exact round trip -------------------------------------------

#[test]
fn cursor_round_trips_bit_exactly() {
    let cursor = Cursor {
        // A score with no short decimal rendering; bit-exactness matters.
        score: f32::from_bits(0x3e99_999a),
        hit_type: HitType::Goal,
        id: Id::new_v4(),
    };
    let decoded = Cursor::decode(&cursor.encode()).expect("round trip decodes");
    assert_eq!(decoded.score.to_bits(), cursor.score.to_bits());
    assert_eq!(decoded.hit_type, cursor.hit_type);
    assert_eq!(decoded.id, cursor.id);
}

#[test]
fn cursor_decode_rejects_garbage() {
    assert_eq!(Cursor::decode("not base64!!"), Err(CursorDecodeError));
    assert_eq!(Cursor::decode(""), Err(CursorDecodeError));
    // Valid base64, wrong length.
    assert_eq!(Cursor::decode("AAAA"), Err(CursorDecodeError));
    // Right length, unknown version byte.
    let mut bytes = vec![9u8];
    bytes.extend_from_slice(&[0u8; 21]);
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    assert_eq!(Cursor::decode(&raw), Err(CursorDecodeError));
}

// --- excerpt_title -----------------------------------------------------------

#[test]
fn excerpt_title_uses_the_body_when_short() {
    assert_eq!(
        excerpt_title(Some("Ship the report"), "Action"),
        "Ship the report"
    );
}

#[test]
fn excerpt_title_truncates_on_a_word_boundary() {
    let body = "sesquipedalian ".repeat(10);
    let title = excerpt_title(Some(&body), "Action");
    assert!(title.chars().count() <= 81, "got: {title}");
    assert!(title.ends_with("sesquipedalian…"), "cut mid-word: {title}");
}

#[test]
fn excerpt_title_falls_back_when_blank() {
    assert_eq!(excerpt_title(None, "Agreement"), "Agreement");
    assert_eq!(excerpt_title(Some("   "), "Agreement"), "Agreement");
}

// --- HitType ordering discipline ---------------------------------------------

#[test]
fn hit_type_order_is_alphabetical_by_wire_name() {
    // The derived Ord is the response sort's "type ASC"; both the merge and the
    // per-searcher cursor fold rely on it matching the wire names' order.
    let mut types = [
        HitType::Topic,
        HitType::CoachingSession,
        HitType::Goal,
        HitType::Action,
        HitType::Agreement,
    ];
    types.sort();
    assert_eq!(
        types,
        [
            HitType::Action,
            HitType::Agreement,
            HitType::CoachingSession,
            HitType::Goal,
            HitType::Topic,
        ]
    );
}
