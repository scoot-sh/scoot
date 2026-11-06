//! Tests for user-owned look targets: schema, tokens and substitution.

use std::collections::BTreeMap;

use super::*;

fn tokens() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("palette.background".to_owned(), "#0E0E0E".to_owned()),
        ("palette.accent".to_owned(), "#FF9A30".to_owned()),
        ("fonts.mono".to_owned(), "FiraCode Nerd Font".to_owned()),
        ("polarity".to_owned(), "dark".to_owned()),
    ])
}

#[test]
fn plain_substitution_replaces_every_token() {
    let out = render_template(
        "bg={{ palette.background }} fg={{palette.accent}}",
        &tokens(),
    )
    .unwrap();
    assert_eq!(out, "bg=#0E0E0E fg=#FF9A30");
}

#[test]
fn template_without_tokens_renders_unchanged() {
    let out = render_template("# static kitty line\nfont_size 12\n", &tokens()).unwrap();
    assert_eq!(out, "# static kitty line\nfont_size 12\n");
}

#[test]
fn stray_close_braces_stay_literal_for_json_nesting() {
    let out = render_template(
        r##"{"colors": {"bg": "{{ palette.background }}"}}"##,
        &tokens(),
    )
    .unwrap();
    assert_eq!(out, r##"{"colors": {"bg": "#0E0E0E"}}"##);
}

#[test]
fn unknown_token_refuses_at_render() {
    let error = render_template("bg={{ palette.backround }}", &tokens()).unwrap_err();
    assert!(error.to_string().contains("unknown token"), "{error}");
}

#[test]
fn unclosed_token_refuses() {
    let error = render_template("bg={{ palette.background", &tokens()).unwrap_err();
    assert!(error.to_string().contains("unclosed"), "{error}");
}

#[test]
fn empty_token_refuses() {
    let error = render_template("bg={{ }}", &tokens()).unwrap_err();
    assert!(error.to_string().contains("empty"), "{error}");
}

#[test]
fn nohash_strips_the_hash() {
    let out = render_template("{{ palette.accent | nohash }}", &tokens()).unwrap();
    assert_eq!(out, "FF9A30");
}

#[test]
fn rgb_expands_to_decimals() {
    let out = render_template("{{ palette.accent | rgb }}", &tokens()).unwrap();
    assert_eq!(out, "255,154,48");
}

#[test]
fn rgba_carries_the_alpha() {
    let out = render_template("{{ palette.accent | rgba(0.5) }}", &tokens()).unwrap();
    assert_eq!(out, "rgba(255,154,48,0.5)");
}

#[test]
fn unknown_filter_refuses() {
    let error = render_template("{{ palette.accent | darken }}", &tokens()).unwrap_err();
    assert!(error.to_string().contains("unknown filter"), "{error}");
}

#[test]
fn filter_on_a_non_color_refuses() {
    let error = render_template("{{ fonts.mono | rgb }}", &tokens()).unwrap_err();
    assert!(
        error.to_string().contains("needs a `#rrggbb` color"),
        "{error}"
    );
}

#[test]
fn rgba_with_a_bad_alpha_refuses() {
    for alpha in ["2", "-0.1", "nan", "half"] {
        let error = render_template(
            &format!("{{{{ palette.accent | rgba({alpha}) }}}}"),
            &tokens(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("0 to 1"), "{error}");
    }
}

#[test]
fn nul_in_template_refuses() {
    let error = render_template("bg={{ palette.background }}\0", &tokens()).unwrap_err();
    assert!(error.to_string().contains("NUL"), "{error}");
}

#[test]
fn oversized_template_refuses() {
    let big = "x".repeat(MAX_TEMPLATE_BYTES + 1);
    let error = render_template(&big, &tokens()).unwrap_err();
    assert!(error.to_string().contains("larger than"), "{error}");
}

#[test]
fn target_toml_parses_minimal() {
    let raw = parse_target_toml(
        br#"name = "kitty-colors"
output = "kitty/colors.conf"
template = "colors.tmpl"
"#,
    )
    .unwrap();
    assert_eq!(raw.name, "kitty-colors");
    assert!(raw.reload.is_none());
}

#[test]
fn target_toml_parses_with_reload() {
    let raw = parse_target_toml(
        br#"name = "kitty-colors"
output = "kitty/colors.conf"
template = "colors.tmpl"

[reload]
argv = ["kitty", "@", "set-colors", "--all"]
timeout_secs = 10
"#,
    )
    .unwrap();
    let reload = raw.reload.unwrap();
    assert_eq!(reload.argv.len(), 4);
    assert_eq!(reload_timeout(&reload), 10);
}

#[test]
fn reload_defaults_to_thirty_seconds() {
    let raw = parse_target_toml(
        br#"name = "kitty-colors"
output = "kitty/colors.conf"
template = "colors.tmpl"

[reload]
argv = ["kitty"]
"#,
    )
    .unwrap();
    assert_eq!(reload_timeout(&raw.reload.unwrap()), 30);
}

#[test]
fn unknown_keys_in_target_toml_refuse() {
    let errors = parse_target_toml(
        br#"name = "kitty-colors"
output = "kitty/colors.conf"
template = "colors.tmpl"
command = "kitty @ set-colors"
"#,
    )
    .unwrap_err();
    assert!(
        errors.iter().any(|e| e.to_string().contains("command")),
        "{errors:?}"
    );
}

#[test]
fn absolute_output_refuses() {
    let mut errors = Vec::new();
    validate_target_shape(
        &RawTarget {
            name: "kitty-colors".to_owned(),
            output: "/etc/kitty.conf".to_owned(),
            template: "colors.tmpl".to_owned(),
            reload: None,
        },
        &mut errors,
    );
    assert!(errors.iter().any(|e| e.field == "output"), "{errors:?}");
}

#[test]
fn dotdot_output_refuses() {
    let mut errors = Vec::new();
    validate_target_shape(
        &RawTarget {
            name: "kitty-colors".to_owned(),
            output: "../escape.conf".to_owned(),
            template: "colors.tmpl".to_owned(),
            reload: None,
        },
        &mut errors,
    );
    assert!(errors.iter().any(|e| e.field == "output"), "{errors:?}");
}

#[test]
fn hidden_output_refuses() {
    let mut errors = Vec::new();
    validate_target_shape(
        &RawTarget {
            name: "kitty-colors".to_owned(),
            output: ".hidden/colors.conf".to_owned(),
            template: "colors.tmpl".to_owned(),
            reload: None,
        },
        &mut errors,
    );
    assert!(errors.iter().any(|e| e.field == "output"), "{errors:?}");
}

#[test]
fn relative_reload_program_refuses() {
    let mut errors = Vec::new();
    validate_target_shape(
        &RawTarget {
            name: "kitty-colors".to_owned(),
            output: "kitty/colors.conf".to_owned(),
            template: "colors.tmpl".to_owned(),
            reload: Some(Reload {
                argv: vec!["./reload.sh".to_owned()],
                timeout_secs: None,
            }),
        },
        &mut errors,
    );
    assert!(
        errors.iter().any(|e| e.field == "reload.argv[0]"),
        "{errors:?}"
    );
}

#[test]
fn name_must_match_its_directory() {
    let raw = RawTarget {
        name: "kitty-colors".to_owned(),
        output: "kitty/colors.conf".to_owned(),
        template: "colors.tmpl".to_owned(),
        reload: None,
    };
    assert!(check_name_matches_dir(&raw, "kitty-colors").is_none());
    assert!(check_name_matches_dir(&raw, "other").is_some());
}

#[test]
fn look_toml_cannot_declare_targets() {
    // Installed looks (someone else's) can never supply templates,
    // output paths or reload commands. `[targets.<name>]` exists now,
    // but holds color, font family and font size overrides only: every smuggling shape
    // refuses at parse, whatever the target name.
    for body in [
        // The target.toml fields themselves.
        "output = \"kitty/colors.conf\"",
        "template = \"colors.tmpl\"",
        "name = \"kitty-colors\"",
        // A reload, as a string and as the real table shape.
        "reload = \"kitty @ set-colors\"",
        // Anything that is not an allowed color, family or size token,
        // including the font tokens that name files or Nix attributes.
        "\"fonts.files.mono\" = \"fonts/x.ttf\"",
        "\"fonts.nix.mono\" = \"pkgs.fira-code\"",
        "\"opacity.bar\" = 0.5",
        "\"icons.theme\" = \"Papirus\"",
        "\"cursor.accent\" = \"#ff0000\"",
        "\"palette.nope\" = \"#ff0000\"",
        "\"terminal.color16\" = \"#ff0000\"",
        "\"terminal.color04\" = \"#ff0000\"",
        "\"terminal.color-1\" = \"#ff0000\"",
    ] {
        let text = format!("schema = 1\nname = \"evil\"\n[targets.kitty-colors]\n{body}\n");
        assert!(
            crate::parse_look_toml(text.as_bytes()).is_err(),
            "must refuse: {body}"
        );
    }
    // A reload sub-table and a non-table target refuse at the type level.
    for text in [
        "schema = 1\nname = \"evil\"\n[targets.kitty-colors.reload]\nargv = [\"sh\"]\n",
        "schema = 1\nname = \"evil\"\n[targets]\nkitty-colors = \"sh -c x\"\n",
    ] {
        assert!(crate::parse_look_toml(text.as_bytes()).is_err(), "{text}");
    }
}

fn override_look(table: &str) -> String {
    format!("schema = 1\nname = \"x\"\n[targets.kitty-colors]\n{table}\n")
}

#[test]
fn color_overrides_parse_and_hostile_values_refuse() {
    let ok =
        override_look("\"palette.background\" = \"#112233\"\n\"terminal.color15\" = \"#aabbcc\"");
    let raw = crate::parse_look_toml(ok.as_bytes()).expect("colors parse");
    assert_eq!(raw.targets["kitty-colors"].len(), 2);
    for value in [
        "\"red\"",
        "\"#12345\"",
        "\"#1234567\"",
        "\"#GGGGGG\"",
        "\"#112233\\n\\nreload = x\"",
        "\"$(reboot)\"",
        "\"{{ palette.accent }}\"",
        "\"\"",
        "7",
        "true",
    ] {
        let text = override_look(&format!("\"palette.accent\" = {value}"));
        assert!(
            crate::parse_look_toml(text.as_bytes()).is_err(),
            "must refuse value {value}"
        );
    }
    // A bad target name refuses at parse; `none`, uppercase, a path.
    for name in ["\"Kitty\"", "\"../x\"", "\"a b\"", "\"\""] {
        let text = format!(
            "schema = 1\nname = \"x\"\n[targets.{name}]\n\"palette.accent\" = \"#112233\"\n"
        );
        assert!(crate::parse_look_toml(text.as_bytes()).is_err(), "{name}");
    }
}

#[test]
fn only_colors_families_and_sizes_are_overridable() {
    use OverrideKind::*;
    for (key, kind) in [
        ("palette.accent-fg", Color),
        ("terminal.color0", Color),
        ("terminal.color15", Color),
        ("fonts.ui", Family),
        ("fonts.sans", Family),
        ("fonts.mono", Family),
        ("fonts.size", Size),
        ("fonts.popup-size", Size),
        ("fonts.app-size", Size),
        ("fonts.terminal-size", Size),
    ] {
        assert_eq!(override_kind(key), Some(kind), "{key}");
    }
    for key in [
        "terminal.color16",
        "terminal.color00",
        "terminal.color+1",
        "terminal.color",
        "fonts",
        "fonts.files",
        "fonts.nix",
        "fonts.files.mono",
        "fonts.mono ",
        "opacity.bar",
        "shape.radius",
        "cursor.theme",
        "icons.theme",
        "wallpaper.color",
        "polarity",
        "palette",
        "palette.",
        "",
    ] {
        assert_eq!(override_kind(key), None, "{key}");
    }
}

fn resolved_with(extra: &str) -> crate::ResolvedLook {
    let text = format!("{}{extra}", crate::schema::tests::minimal_toml());
    let raw = crate::parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("moonrise".to_string(), (raw, crate::Origin::User))]);
    crate::resolve(&map, "moonrise").expect("resolves")
}

#[test]
fn overrides_replace_only_for_their_own_target() {
    let look = resolved_with(
        "\n[targets.kitty-colors]\n\"palette.background\" = \"#010203\"\n\"terminal.color3\" = \"#0a0b0c\"\n",
    );
    let base = target_tokens(&look);
    assert!(!base.contains_key("terminal.color3"), "no [terminal] table");
    let kitty = tokens_for_target(&base, &look, "kitty-colors");
    assert_eq!(kitty["palette.background"], "#010203");
    assert_eq!(kitty["terminal.color3"], "#0a0b0c", "adds a missing color");
    assert_eq!(kitty["palette.accent"], base["palette.accent"]);
    // A target the look does not mention sees the plain tokens, borrowed.
    let other = tokens_for_target(&base, &look, "tmux");
    assert!(matches!(other, std::borrow::Cow::Borrowed(_)));
    assert_eq!(*other, base);
    assert_ne!(base["palette.background"], "#010203");
}

#[test]
fn a_look_without_the_table_resolves_to_no_overrides() {
    let look = resolved_with("");
    assert!(look.target_overrides.is_empty());
    let base = target_tokens(&look);
    assert!(matches!(
        tokens_for_target(&base, &look, "kitty-colors"),
        std::borrow::Cow::Borrowed(_)
    ));
}

#[test]
fn a_child_look_merges_overrides_per_key_over_its_base() {
    let base_text = format!(
        "{}\n[targets.kitty-colors]\n\"palette.background\" = \"#010203\"\n\"palette.accent\" = \"#040506\"\n",
        crate::schema::tests::minimal_toml()
    );
    let base = crate::parse_look_toml(base_text.as_bytes()).expect("base");
    let child_text = "schema = 1\nname = \"child\"\nextends = \"base\"\n[targets.kitty-colors]\n\"palette.accent\" = \"#070809\"\n[targets.tmux]\n\"palette.dim\" = \"#0a0a0a\"\n";
    let child = crate::parse_look_toml(child_text.as_bytes()).expect("child");
    let map = BTreeMap::from([
        ("base".to_string(), (base, crate::Origin::User)),
        ("child".to_string(), (child, crate::Origin::User)),
    ]);
    let look = crate::resolve(&map, "child").expect("resolves");
    let kitty = &look.target_overrides["kitty-colors"];
    assert_eq!(kitty["palette.background"], "#010203", "inherited");
    assert_eq!(kitty["palette.accent"], "#070809", "child wins per key");
    assert_eq!(look.target_overrides["tmux"]["palette.dim"], "#0a0a0a");
}

#[test]
fn look_toml_cannot_allowlist_a_target_app() {
    // `[apps.kitty]` is not an app any loader reads, so the allowlist
    // refuses it by name: looks cannot sneak target-shaped overrides in
    // through the per-app tables either.
    let errors = crate::parse_look_toml(
        br##"schema = 1
name = "evil"

[apps.kitty]
background = "#000000"
"##,
    )
    .unwrap_err();
    assert!(
        errors.iter().any(|e| e.to_string().contains("kitty")),
        "{errors:?}"
    );
}

#[test]
fn nfc_output_refuses_as_non_ascii() {
    // Security review of PR #871, finding 3: `caf\u{e9}/x.conf` (NFC)
    // and `cafe\u{301}/x.conf` (NFD) are one file on APFS but two byte
    // strings here, so the second would stage silently past a byte
    // compare. No normalization crate sits in the workspace lock tree
    // (`unicode-normalization` is absent from `Cargo.lock`), so
    // non-ASCII outputs refuse at parse instead of reaching the
    // duplicate check at all.
    let mut errors = Vec::new();
    validate_target_shape(
        &RawTarget {
            name: "kitty-colors".to_owned(),
            output: "caf\u{e9}/x.conf".to_owned(),
            template: "colors.tmpl".to_owned(),
            reload: None,
        },
        &mut errors,
    );
    assert!(
        errors.iter().any(|e| e.field == "output"),
        "NFC output must refuse: {errors:?}"
    );
}

#[test]
fn nfd_output_refuses_as_non_ascii() {
    // The NFD half of the same pair: `e` plus a combining acute. Refuses
    // like its NFC twin, so neither spelling can stage, let alone both.
    let mut errors = Vec::new();
    validate_target_shape(
        &RawTarget {
            name: "kitty-colors".to_owned(),
            output: "cafe\u{301}/x.conf".to_owned(),
            template: "colors.tmpl".to_owned(),
            reload: None,
        },
        &mut errors,
    );
    assert!(
        errors.iter().any(|e| e.field == "output"),
        "NFD output must refuse: {errors:?}"
    );
}

#[test]
fn unicode_template_stays_allowed() {
    // The ASCII rule is outputs only: a template is a file inside the
    // target's own directory, read `O_NOFOLLOW`, with no duplicate
    // comparison run over it, so a Unicode template name keeps working.
    let mut errors = Vec::new();
    validate_target_shape(
        &RawTarget {
            name: "kitty-colors".to_owned(),
            output: "kitty/colors.conf".to_owned(),
            template: "caf\u{e9}.tmpl".to_owned(),
            reload: None,
        },
        &mut errors,
    );
    assert!(
        errors.is_empty(),
        "a Unicode template must pass: {errors:?}"
    );
}

fn refuses(table: &str) -> bool {
    crate::parse_look_toml(override_look(table).as_bytes()).is_err()
}

#[test]
fn font_family_overrides_parse_with_the_fonts_table_grammar() {
    let ok = override_look(
        "\"fonts.mono\" = \"JetBrains Mono\"\n\"fonts.sans\" = \"Inter\"\n\"fonts.ui\" = \"A-b_c.d+e 9\"",
    );
    let raw = crate::parse_look_toml(ok.as_bytes()).expect("families parse");
    assert_eq!(raw.targets["kitty-colors"].len(), 3);
    // The longest the look's own [fonts] accepts (64 bytes) is accepted.
    let longest = "a".repeat(64);
    assert!(!refuses(&format!("\"fonts.mono\" = \"{longest}\"")));
}

#[test]
fn hostile_font_families_refuse_one_injection_shape_at_a_time() {
    // Each shape is a way to end the font value and start another config
    // line (kitty/alacritty/tmux/WezTerm/Ghostty), break out of a quoted
    // template context, or reach the template engine or a path.
    let hostile: &[(&str, &str)] = &[
        ("newline", "\"Fira\\nreload = x\""),
        ("carriage return", "\"Fira\\rinclude evil.conf\""),
        ("trailing newline", "\"Fira\\n\""),
        ("NUL", "\"Fira\\u0000\""),
        ("tab", "\"Fira\\tMono\""),
        ("ESC", "\"Fira\\u001b[2J\""),
        ("DEL", "\"Fira\\u007f\""),
        ("C1 control", "\"Fira\\u0085Mono\""),
        ("line separator", "\"Fira\\u2028Mono\""),
        ("paragraph separator", "\"Fira\\u2029Mono\""),
        ("bidi override", "\"Fira\\u202eMono\""),
        ("bidi isolate", "\"Fira\\u2066Mono\""),
        ("zero-width space", "\"Fira\\u200bMono\""),
        ("BOM", "\"\\ufeffFira\""),
        ("non-ASCII family", "\"Fira\\u00e9\""),
        ("double quote", "\"Fira\\\"; exec = x\""),
        ("single quote", "\"Fira'\""),
        ("backtick", "\"Fira`id`\""),
        ("backslash", "\"Fira\\\\\""),
        ("hash comment", "\"Fira # x\""),
        ("semicolon", "\"Fira; x\""),
        ("equals", "\"Fira = x\""),
        ("square bracket", "\"Fira]\""),
        ("brace", "\"Fira}\""),
        ("template open", "\"{{ palette.accent }}\""),
        ("template close", "\"Fira }}\""),
        ("shell expansion", "\"$(reboot)\""),
        ("env expansion", "\"${HOME}\""),
        ("percent", "\"Fira%n\""),
        ("path", "\"../../etc/passwd\""),
        ("absolute path", "\"/usr/share/fonts/x.ttf\""),
        ("slash", "\"a/b\""),
        ("leading dot", "\".hidden\""),
        ("leading space", "\" Fira\""),
        ("leading hyphen", "\"-Fira\""),
        ("empty", "\"\""),
        (
            "too long",
            "\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"",
        ),
        ("number", "12"),
        ("boolean", "true"),
        ("array", "[\"Fira\"]"),
    ];
    for key in ["fonts.mono", "fonts.sans", "fonts.ui"] {
        for (shape, value) in hostile {
            assert!(
                refuses(&format!("\"{key}\" = {value}")),
                "{key}: {shape} must refuse"
            );
        }
    }
}

#[test]
fn font_size_overrides_take_a_number_in_the_fonts_table_range() {
    for value in ["4", "4.0", "12", "12.5", "192", "192.0"] {
        for key in [
            "fonts.size",
            "fonts.popup-size",
            "fonts.app-size",
            "fonts.terminal-size",
        ] {
            assert!(!refuses(&format!("\"{key}\" = {value}")), "{key} = {value}");
        }
    }
    for value in [
        "3.999",
        "0",
        "-12",
        "192.001",
        "1e9",
        "nan",
        "inf",
        "-inf",
        "\"12\"",
        "\"12pt\"",
        "\"12\\nreload = x\"",
        "\"\"",
        "true",
        "[12]",
    ] {
        assert!(
            refuses(&format!("\"fonts.terminal-size\" = {value}")),
            "{value}"
        );
    }
}

#[test]
fn a_color_key_refuses_a_family_or_a_number_and_a_family_key_refuses_a_color_free_pass() {
    assert!(refuses("\"palette.accent\" = \"Fira Code\""));
    assert!(refuses("\"palette.accent\" = 12"));
    assert!(refuses("\"fonts.size\" = \"#112233\""));
    // A color is a legal identifier shape only without the `#`.
    assert!(refuses("\"fonts.mono\" = \"#112233\""));
}

#[test]
fn font_overrides_replace_only_for_their_own_target_and_render_like_the_fonts_table() {
    let look = resolved_with(
        "\n[targets.kitty-colors]\n\"fonts.mono\" = \"Iosevka\"\n\"fonts.terminal-size\" = 13\n\"fonts.size\" = 10.5\n",
    );
    let base = target_tokens(&look);
    let kitty = tokens_for_target(&base, &look, "kitty-colors");
    assert_eq!(kitty["fonts.mono"], "Iosevka");
    assert_eq!(
        kitty["fonts.terminal-size"], "13",
        "no `.0`, like the table"
    );
    assert_eq!(kitty["fonts.size"], "10.5");
    assert_eq!(kitty["fonts.sans"], base["fonts.sans"], "untouched");
    assert_ne!(base["fonts.mono"], "Iosevka");
    let other = tokens_for_target(&base, &look, "tmux");
    assert!(matches!(other, std::borrow::Cow::Borrowed(_)));
    assert_eq!(*other, base);
}

#[test]
fn overriding_every_font_token_leaves_the_looks_own_fonts_untouched() {
    let plain = resolved_with("");
    let look = resolved_with(
        "\n[targets.kitty-colors]\n\"fonts.ui\" = \"UiOver\"\n\"fonts.sans\" = \"SansOver\"\n\"fonts.mono\" = \"MonoOver\"\n\"fonts.size\" = 31\n\"fonts.popup-size\" = 32\n\"fonts.app-size\" = 33\n\"fonts.terminal-size\" = 34\n",
    );
    // Every override really is set (so the comparison below is not vacuous).
    let table = &look.target_overrides["kitty-colors"];
    assert_eq!(table.len(), 7);
    let (a, b) = (&look.fonts, &plain.fonts);
    assert_eq!(a.ui, b.ui);
    assert_eq!(a.sans, b.sans);
    assert_eq!(a.mono, b.mono);
    assert_eq!(a.size, b.size);
    assert_eq!(a.popup_size, b.popup_size);
    assert_eq!(a.app_size, b.app_size);
    assert_eq!(a.terminal_size, b.terminal_size);
    for (value, own) in [
        ("UiOver", &a.ui),
        ("SansOver", &a.sans),
        ("MonoOver", &a.mono),
    ] {
        assert_ne!(own, value);
    }
    // And the base tokens the other targets see are the plain ones.
    assert_eq!(target_tokens(&look), target_tokens(&plain));
}

#[test]
fn a_wrong_value_type_names_what_is_allowed() {
    for value in ["true", "[1]", "{ a = 1 }", "2020-01-01"] {
        let err =
            crate::parse_look_toml(override_look(&format!("\"fonts.size\" = {value}")).as_bytes())
                .unwrap_err();
        let text = format!("{err:?}");
        assert!(
            text.contains("a quoted string (colors, font families) or a number (font sizes)"),
            "{value}: {text}"
        );
        assert!(!text.contains("untagged"), "{value}: {text}");
    }
}

#[test]
fn a_child_look_merges_font_overrides_per_key() {
    let base_text = format!(
        "{}\n[targets.kitty-colors]\n\"fonts.mono\" = \"Iosevka\"\n\"fonts.terminal-size\" = 11\n",
        crate::schema::tests::minimal_toml()
    );
    let base = crate::parse_look_toml(base_text.as_bytes()).expect("base");
    let child_text = "schema = 1\nname = \"child\"\nextends = \"base\"\n[targets.kitty-colors]\n\"fonts.terminal-size\" = 14.5\n";
    let child = crate::parse_look_toml(child_text.as_bytes()).expect("child");
    let map = BTreeMap::from([
        ("base".to_string(), (base, crate::Origin::User)),
        ("child".to_string(), (child, crate::Origin::User)),
    ]);
    let look = crate::resolve(&map, "child").expect("resolves");
    let kitty = &look.target_overrides["kitty-colors"];
    assert_eq!(kitty["fonts.mono"], "Iosevka", "inherited");
    assert_eq!(kitty["fonts.terminal-size"], "14.5", "child wins per key");
}
