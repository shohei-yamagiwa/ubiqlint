# tree-sitter-java (patched for ubiqlint)

This directory vendors [tree-sitter-java](https://github.com/tree-sitter/tree-sitter-java)
0.23.5 with a patch to `grammar.js` for ubiqlint.

## Patch

Java allows modifiers in type patterns and record pattern components
(JLS 14.30.1), but the grammar in upstream version 0.23.5 does not, so these
are syntax errors in that version:

```java
case final String s -> 1;
case @Deprecated String s -> 1;
if (o instanceof Point(final int x, int y)) {}
```

`type_pattern` and `record_pattern_component` now start with
`optional($.modifiers)`. The changed lines are marked with `ubiqlint:` in
`grammar.js`. `test/corpus/ubiqlint.txt` covers them; the other files in
`test/corpus` are the upstream tests.

## Regenerating

Everything in `src/` is generated. After changing `grammar.js`, run the
tree-sitter CLI 0.26 in this directory:

```sh
tree-sitter generate
tree-sitter test
```

The Rust bindings in `bindings/rust` are trimmed to what ubiqlint uses.

## License

MIT, as upstream. See `LICENSE`.
