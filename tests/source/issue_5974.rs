macro_rules! repro {
    () => {
        #[doc = concat!("let var = ",
                        "false;")]
        fn f() {}
    };
}

// `#[rustfmt::skip]` inside a macro body must disable the non-idempotent
// indent strip (the probe in `macro_def_body_snippet_has_skip_attribute`),
// so the hand-crafted layout below survives formatting unchanged.
macro_rules! repro_skip_path {
    () => {
        #[rustfmt::skip]
        fn layout() {
            let x = [ 1,2,3,
                      4,5,6 ];
        }
    };
}

// Same guarantee for the deprecated `#[rustfmt_skip]` form.
macro_rules! repro_skip_depr {
    () => {
        #[rustfmt_skip]
        fn layout() {
            let x = [ 1,2,3,
                      4,5,6 ];
        }
    };
}

// And for the `cfg_attr`-nested form.
macro_rules! repro_skip_cfg_attr {
    () => {
        #[cfg_attr(rustfmt, rustfmt_skip)]
        fn layout() {
            let x = [ 1,2,3,
                      4,5,6 ];
        }
    };
}

// Nested macros with the skip attribute on the inner block. The outer body
// itself has no skip, so the outer-rewrite's probe must NOT match the inner
// macro's opaque token stream (which the AST visitor cannot descend into).
// When the inner is recursively formatted, its own probe runs on the inner's
// body and finds the skip, preserving the irregular fn layout.
macro_rules! repro_skip_nested {
    () => {
        macro_rules! inner_skip {
            () => {
                #[rustfmt::skip]
                fn ugly() {
                    let x = [ 1,2,3,
                              4,5,6 ];
                }
            };
        }
    };
}
