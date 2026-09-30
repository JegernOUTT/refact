import contextlib
import io
import tempfile
import unittest
from pathlib import Path

import fn_size_report


class FunctionSizeReportTests(unittest.TestCase):
    def test_lexer_ignores_raw_strings_comments_and_char_literals(self):
        source = r'''fn kept<'a>(value: &'a str) {
    let raw = r###"{ ignored } and \"#"###;
    let bytes = br#"{ also ignored }"#;
    // { ignored
    /* outer { /* nested } */ still comment } */
    let left = '{';
    let right = '}';
    let quote = '\'';
    let unicode = '\u{7b}';
}
'''
        functions = fn_size_report.functions_in_source(source, "fixture.rs")
        self.assertEqual([(function.name, function.lines) for function in functions], [("kept", 10)])

    def test_adjacent_literals_comments_and_lifetimes_do_not_corrupt_delimiters(self):
        source = r'''fn kept<'a, 'static>() {
    let a=&"{";
    let b=x,"{";
    let c=&r#"{ "#;
    let d=br#"{"#;
    let e='"';
    let f=b'{';
    foo// {
    bar/* { */
    let _borrow = &'a { };
}
'''
        functions = fn_size_report.functions_in_source(source, "fixture.rs")
        self.assertEqual([(function.name, function.lines) for function in functions], [("kept", 11)])

    def test_literal_prefixes_are_not_recognized_inside_identifiers(self):
        source = 'fn kept() { let text = xbr#"{"#; let raw = xr#"{"#; }\n'
        functions = fn_size_report.functions_in_source(source, "fixture.rs")
        self.assertEqual([(function.name, function.lines) for function in functions], [("kept", 1)])

    def test_nested_functions_are_found_but_closures_are_not(self):
        source = """fn outer() {
    let closure = || {
        if true { 1 }
    };
    fn inner() {}
}
"""
        functions = fn_size_report.functions_in_source(source, "nested.rs")
        self.assertEqual([function.name for function in functions], ["outer", "inner"])
        self.assertEqual([function.lines for function in functions], [6, 1])

    def test_cfg_test_module_is_excluded(self):
        source = """fn production() {}
#[cfg(test)]
mod tests {
    fn hidden() {}
}
"""
        self.assertEqual(
            [function.name for function in fn_size_report.functions_in_source(source, "lib.rs")],
            ["production"],
        )

    def test_cfg_test_module_allows_intervening_attributes(self):
        source = """#[cfg(test)]
#[allow(dead_code)]
/// tests live here
mod tests { fn hidden() {} }
#[cfg(all(test, unix))]
mod platform_tests { fn also_hidden() {} }
fn production() {}
"""
        self.assertEqual(
            [function.name for function in fn_size_report.functions_in_source(source, "lib.rs")],
            ["production"],
        )

    def test_test_attribute_functions_are_excluded(self):
        source = """#[test]
fn regular_test() {}
#[tokio::test]
async fn async_test() {}
#[framework::test]
fn framework_test() {}
fn production() {}
"""
        self.assertEqual(
            [function.name for function in fn_size_report.functions_in_source(source, "lib.rs")],
            ["production"],
        )

    def test_impl_methods_are_qualified(self):
        source = """impl<T> Tool<T> {
    pub(crate) async fn run(&self) {}
}
"""
        functions = fn_size_report.functions_in_source(source, "impl.rs")
        self.assertEqual([function.name for function in functions], ["run"])

    def test_test_files_and_tests_directories_are_excluded(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "src"
            (root / "nested" / "tests").mkdir(parents=True)
            (root / "lib.rs").write_text("fn kept() {}\n", encoding="utf-8")
            (root / "tests.rs").write_text("fn skipped() {}\n", encoding="utf-8")
            (root / "module_tests.rs").write_text("fn skipped_too() {}\n", encoding="utf-8")
            (root / "nested" / "tests" / "fixture.rs").write_text("fn skipped_dir() {}\n", encoding="utf-8")
            count, functions = fn_size_report.scan_roots([root], root.parent)
        self.assertEqual(count, 1)
        self.assertEqual([function.name for function in functions], ["kept"])

    def test_out_of_line_cfg_test_module_file_is_excluded(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "src"
            root.mkdir()
            (root / "lib.rs").write_text(
                "#[cfg(test)]\nmod helpers;\nfn kept() {}\n", encoding="utf-8"
            )
            (root / "helpers.rs").write_text("fn hidden() {}\n", encoding="utf-8")
            count, functions = fn_size_report.scan_roots([root], root.parent)
        self.assertEqual(count, 1)
        self.assertEqual([function.name for function in functions], ["kept"])

    def test_threshold_and_top_are_union_and_advisory(self):
        source = "\n".join(
            ["fn largest() {"] + ["    let _ = 1;" for _ in range(8)] + ["}", "fn medium() {", "    let _ = 1;", "}", "fn small() {}"]
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "src"
            root.mkdir()
            (root / "lib.rs").write_text(source, encoding="utf-8")
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                result = fn_size_report.main(["--root", str(root), "--threshold", "2", "--top", "1"])
        self.assertEqual(result, 0)
        report = output.getvalue()
        self.assertIn("Scanned 1 Rust files; threshold: 2 lines", report)
        self.assertIn("largest", report)
        self.assertIn("medium", report)
        self.assertNotIn("  small\n", report)
        self.assertIn("2 functions over 2 lines (advisory)", report)

    def test_all_functions_over_threshold_are_shown_and_return_success(self):
        source = "\n".join(["fn first() {"] + ["    let _ = 1;" for _ in range(3)] + ["}", "fn second() {"] + ["    let _ = 2;" for _ in range(3)] + ["}"])
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "src"
            root.mkdir()
            (root / "lib.rs").write_text(source, encoding="utf-8")
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                result = fn_size_report.main(["--root", str(root), "--threshold", "2", "--top", "0"])
        self.assertEqual(result, 0)
        self.assertIn("first", output.getvalue())
        self.assertIn("second", output.getvalue())
        self.assertIn("2 functions over 2 lines", output.getvalue())

    def test_missing_root_is_a_usage_error(self):
        with self.assertRaises(SystemExit) as result:
            fn_size_report.main(["--root", "/path/that/does/not/exist"])
        self.assertEqual(result.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
