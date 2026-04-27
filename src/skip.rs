//! Module that contains skip related stuffs.

use rustc_ast::ast;
use rustc_ast::visit::{self, Visitor};
use rustc_ast_pretty::pprust;
use std::collections::HashSet;

use crate::config::Config;
use crate::parse::parser::Parser;
use crate::parse::session::ParseSess;
use crate::utils::contains_skip;

/// Track which blocks of code are to be skipped when formatting.
///
/// You can update it by:
///
/// - attributes slice
/// - manually feeding values into the underlying contexts
///
/// Query this context to know if you need to skip a block.
#[derive(Default, Clone)]
pub(crate) struct SkipContext {
    pub(crate) macros: SkipNameContext,
    pub(crate) attributes: SkipNameContext,
}

impl SkipContext {
    pub(crate) fn update_with_attrs(&mut self, attrs: &[ast::Attribute]) {
        self.macros.extend(get_skip_names("macros", attrs));
        self.attributes.extend(get_skip_names("attributes", attrs));
    }

    pub(crate) fn update(&mut self, other: SkipContext) {
        let SkipContext { macros, attributes } = other;
        self.macros.update(macros);
        self.attributes.update(attributes);
    }
}

/// Track which names to skip.
///
/// Query this context with a string to know whether to skip it.
#[derive(Clone)]
pub(crate) enum SkipNameContext {
    All,
    Values(HashSet<String>),
}

impl Default for SkipNameContext {
    fn default() -> Self {
        Self::Values(Default::default())
    }
}

impl Extend<String> for SkipNameContext {
    fn extend<T: IntoIterator<Item = String>>(&mut self, iter: T) {
        match self {
            Self::All => {}
            Self::Values(values) => values.extend(iter),
        }
    }
}

impl SkipNameContext {
    pub(crate) fn update(&mut self, other: Self) {
        match (self, other) {
            // If we're already skipping everything, nothing more can be added
            (Self::All, _) => {}
            // If we want to skip all, set it
            (this, Self::All) => {
                *this = Self::All;
            }
            // If we have some new values to skip, add them
            (Self::Values(existing_values), Self::Values(new_values)) => {
                existing_values.extend(new_values)
            }
        }
    }

    pub(crate) fn skip(&self, name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Values(values) => values.contains(name),
        }
    }

    pub(crate) fn skip_all(&mut self) {
        *self = Self::All;
    }
}

static RUSTFMT: &str = "rustfmt";
static SKIP: &str = "skip";

/// Say if you're playing with `rustfmt`'s skip attribute
pub(crate) fn is_skip_attr(segments: &[ast::PathSegment]) -> bool {
    if segments.len() < 2 || segments[0].ident.to_string() != RUSTFMT {
        return false;
    }
    match segments.len() {
        2 => segments[1].ident.to_string() == SKIP,
        3 => {
            segments[1].ident.to_string() == SKIP
                && ["macros", "attributes"]
                    .iter()
                    .any(|&n| n == pprust::path_segment_to_string(&segments[2]))
        }
        _ => false,
    }
}

/// Whether `snippet` contains a rustfmt skip attribute (`rustfmt_skip`, `rustfmt::skip`, nested
/// `cfg_attr(.., ...)`, etc.), using the same predicates as [`crate::utils::contains_skip`] on real
/// AST attributes.
///
/// The snippet is wrapped as a function body so it can contain statements (e.g. attributed
/// expressions inside a `macro_rules` arm body). Returns `false` when the snippet does not parse so
/// callers fall back to normal formatting.
///
/// The probe runs inside a disposable [`ParseSess`] so its diagnostics, `can_reset_errors` flag,
/// and `SourceMap` entries cannot leak into the caller's main parsing session.
pub(crate) fn macro_def_body_snippet_has_skip_attribute(config: &Config, snippet: &str) -> bool {
    let mut probe_config = config.clone();
    probe_config.set().show_parse_errors(false);
    let Ok(probe_psess) = ParseSess::new(&probe_config) else {
        return false;
    };

    let wrapped = format!("fn __rustfmt_macro_body_probe() {{\n{snippet}\n}}\n");
    let Ok(krate) = Parser::parse_crate(crate::Input::Text(wrapped), &probe_psess) else {
        return false;
    };
    let mut probe = SkipAttrProbe::default();
    visit::walk_crate(&mut probe, &krate);
    probe.found
}

#[derive(Default)]
struct SkipAttrProbe {
    found: bool,
}

impl<'ast> Visitor<'ast> for SkipAttrProbe {
    fn visit_attribute(&mut self, attr: &'ast ast::Attribute) {
        if self.found {
            return;
        }
        if contains_skip(std::slice::from_ref(attr)) {
            self.found = true;
        }
    }
}

fn get_skip_names(kind: &str, attrs: &[ast::Attribute]) -> Vec<String> {
    let mut skip_names = vec![];
    let path = format!("{RUSTFMT}::{SKIP}::{kind}");
    for attr in attrs {
        // rustc_ast::ast::Path is implemented partialEq
        // but it is designed for segments.len() == 1
        if let ast::AttrKind::Normal(normal) = &attr.kind {
            if pprust::path_to_string(&normal.item.path) != path {
                continue;
            }
        }

        if let Some(list) = attr.meta_item_list() {
            for meta_item_inner in list {
                if let Some(name) = meta_item_inner.ident() {
                    skip_names.push(name.to_string());
                }
            }
        }
    }
    skip_names
}

#[cfg(test)]
mod probe_tests {
    //! Tests for [`macro_def_body_snippet_has_skip_attribute`]. These run inside
    //! [`rustc_span::create_session_if_not_set_then`] because `Parser::parse_crate` reads
    //! thread-local session globals (symbol interner, source map) that rustc sets up around its
    //! parser. A bare `#[test]` without that setup panics with
    //! `cannot access a scoped thread local variable without calling 'set' first`.
    use super::*;
    use crate::config::{Config, Edition};

    fn with_session<R>(f: impl FnOnce() -> R) -> R {
        rustc_span::create_session_if_not_set_then(Edition::Edition2021.into(), |_| f())
    }

    fn has_skip(snippet: &str) -> bool {
        with_session(|| {
            let mut cfg = Config::default();
            cfg.set().show_parse_errors(false);
            macro_def_body_snippet_has_skip_attribute(&cfg, snippet)
        })
    }

    #[test]
    fn finds_rustfmt_path_skip_on_fn_item() {
        assert!(has_skip("#[rustfmt::skip] fn f() {}"));
    }

    #[test]
    fn finds_deprecated_rustfmt_skip_on_fn_item() {
        assert!(has_skip("#[rustfmt_skip] fn f() {}"));
    }

    #[test]
    fn finds_cfg_attr_rustfmt_skip_on_fn_item() {
        assert!(has_skip("#[cfg_attr(rustfmt, rustfmt_skip)] fn f() {}"));
    }

    #[test]
    fn finds_cfg_attr_rustfmt_skip_on_stmt_expr() {
        // Mirrors the shape of `tests/target/issue-3105.rs`: a skip attribute applied to a
        // statement expression inside a macro arm body.
        assert!(has_skip(
            "#[cfg_attr(rustfmt, rustfmt_skip)]\n\
             simd_shuffle(a, a, [0, 1, 2, 3]);"
        ));
    }

    #[test]
    fn ignores_skip_looking_strings_and_comments() {
        // String literals and comments containing the magic words must NOT trip the probe,
        // only real attributes do. This is exactly what motivated replacing the prior
        // `body_str.contains("rustfmt_skip")` substring heuristic.
        assert!(!has_skip(r#"fn f() { let s = "rustfmt_skip"; }"#));
        assert!(!has_skip(r#"fn f() { let s = "rustfmt::skip"; }"#));
        assert!(!has_skip("// rustfmt_skip\nfn f() {}"));
        assert!(!has_skip("/* rustfmt::skip */\nfn f() {}"));
    }

    #[test]
    fn returns_false_for_unrelated_attributes() {
        assert!(!has_skip("#[inline] fn f() {}"));
        assert!(!has_skip("#[doc = \"hi\"] fn f() {}"));
    }

    #[test]
    fn returns_false_for_unparseable_snippet() {
        assert!(!has_skip("this is not valid rust {{{"));
    }

    #[test]
    fn does_not_descend_into_inner_macro_token_tree() {
        // A `macro_rules!` body is parsed into `ItemKind::MacroDef` whose body is an opaque
        // `TokenStream`. Our `Visitor` only sees AST attributes; it must NOT match a skip
        // attribute that lives inside an inner macro's body. Otherwise an outer macro_rules
        // rewrite would erroneously skip its own indent normalization just because some inner
        // macro happens to expand a skipped item. The recursive rewrite of the inner macro is
        // where that skip should be observed instead.
        let snippet = "\
            macro_rules! inner {\n\
                () => {\n\
                    #[rustfmt::skip]\n\
                    fn ugly() { let x = [1, 2]; }\n\
                };\n\
            }\n\
        ";
        assert!(!has_skip(snippet));
    }

    #[test]
    fn finds_skip_inside_inner_macro_body_when_probed_directly() {
        // Once the inner macro_rules body is itself the snippet (the recursive call from
        // `MacroBranch::rewrite` for the inner arm), the same probe must find the skip.
        let snippet = "\
            #[rustfmt::skip]\n\
            fn ugly() { let x = [1, 2]; }\n\
        ";
        assert!(has_skip(snippet));
    }
}
