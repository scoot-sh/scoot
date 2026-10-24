//! The static terminal leaves: shape checks over fixed contents.

use super::*;

const HELIX: &str =
    "\"ui.background\" = {}\n\"ui.text\" = \"#FF0000\"\n\"ui.selection\" = \"#FF0000\"\n";
const BTOP: &str = "# a theme\ntheme[main_fg]=\"#FF0000\"\ntheme[title]=\"#00FF00\"\n";
const STARSHIP: &str = "add_newline = false\nformat = \"$directory$character\"\n";
const NVIM: &str = "-- a colorscheme\nvim.g.colors_name = \"scoot-look\"\n";

#[test]
fn shipped_shapes_pass() {
    check_static_leaf("apps/helix.toml", HELIX.as_bytes()).expect("helix theme");
    check_static_leaf("apps/btop.theme", BTOP.as_bytes()).expect("btop theme");
    check_static_leaf("apps/starship.toml", STARSHIP.as_bytes()).expect("starship config");
    check_static_leaf("apps/nvim.lua", NVIM.as_bytes()).expect("nvim colorscheme");
}

#[test]
fn unknown_leaf_refuses() {
    let error = check_static_leaf("apps/foot.ini", b"x").expect_err("not a static leaf");
    assert!(error.to_string().contains("not a static leaf"), "{error}");
}

#[test]
fn oversized_leaf_refuses() {
    let big = vec![b'#'; MAX_STATIC_BYTES as usize + 1];
    let error = check_static_leaf("apps/btop.theme", &big).expect_err("oversized");
    assert!(error.to_string().contains("larger than"), "{error}");
}

#[test]
fn non_utf8_and_nul_refuse() {
    let error = check_static_leaf("apps/helix.toml", &[0xff, 0xfe]).expect_err("non-UTF-8");
    assert!(error.to_string().contains("UTF-8"), "{error}");
    let error = check_static_leaf("apps/starship.toml", b"a = \"b\0c\"\n").expect_err("NUL");
    assert!(error.to_string().contains("NUL"), "{error}");
}

#[test]
fn broken_toml_refuses() {
    // A truncated theme (the mutation this guards: a half-written leaf)
    // must not render.
    let error = check_static_leaf("apps/helix.toml", b"\"ui.text\" = ").expect_err("truncated");
    assert!(error.to_string().contains("TOML"), "{error}");
}

#[test]
fn btop_shape_refuses() {
    // A bare word is no theme entry.
    let error = check_static_leaf("apps/btop.theme", b"hello\n").expect_err("bare word");
    assert!(error.to_string().contains("not a theme entry"), "{error}");
    // Comments alone are no theme.
    let error =
        check_static_leaf("apps/btop.theme", b"# only comments\n\n").expect_err("no entries");
    assert!(error.to_string().contains("no theme entries"), "{error}");
    // A missing value is no entry.
    let error =
        check_static_leaf("apps/btop.theme", b"theme[main_fg]=\n").expect_err("empty value");
    assert!(error.to_string().contains("not a theme entry"), "{error}");
    // A carriage return would write a broken line into the render.
    let error = check_static_leaf("apps/btop.theme", b"theme[a]=\"b\"\r\n").expect_err("CR");
    assert!(error.to_string().contains("carriage returns"), "{error}");
}

#[test]
fn leaf_table_shapes() {
    assert_eq!(STATIC_LEAVES.len(), 4);
    assert!(leaf_for("apps/helix.toml").is_some());
    assert!(leaf_for("apps/btop.theme").is_some());
    assert!(leaf_for("apps/starship.toml").is_some());
    assert!(leaf_for("apps/nvim.lua").is_some());
    assert!(leaf_for("apps/foot.ini").is_none());
}

#[test]
fn lua_shape_refuses() {
    // A Lua file that never names itself is not a colorscheme.
    let error =
        check_static_leaf("apps/nvim.lua", b"vim.cmd(\"hi clear\")\n").expect_err("no name");
    assert!(error.to_string().contains("colors_name"), "{error}");
    // A carriage return would land verbatim in the render.
    let error =
        check_static_leaf("apps/nvim.lua", b"vim.g.colors_name = \"x\"\r\n").expect_err("CR");
    assert!(error.to_string().contains("carriage returns"), "{error}");
}
