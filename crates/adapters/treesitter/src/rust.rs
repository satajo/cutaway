use cutaway_architecture::{ElementKind, ElementName};
use cutaway_inspection::ports::source_tree::{SourceFile, SourcePath};
use cutaway_inspection::ports::syntax_analyzer::{
    Declaration, SyntaxAnalysisError, SyntaxAnalyzer,
};

/// Understands `.rs` files: declares the top-level functions, types, and
/// inline modules of a file. Nested items stay undeclared until the
/// architecture model grows nested containment.
pub struct RustSyntaxAnalyzer;

impl SyntaxAnalyzer for RustSyntaxAnalyzer {
    fn supports(&self, path: &SourcePath) -> bool {
        path.extension() == Some("rs")
    }

    fn analyze(&self, file: &SourceFile) -> Result<Vec<Declaration>, SyntaxAnalysisError> {
        let text =
            std::str::from_utf8(&file.contents).map_err(|_| SyntaxAnalysisError::NonUtf8Text {
                path: file.path.clone(),
            })?;

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("the bundled Rust grammar matches the linked tree-sitter version");
        let tree = parser
            .parse(text, None)
            .ok_or_else(|| unparseable(&file.path, "the parser produced no syntax tree"))?;
        if tree.root_node().has_error() {
            return Err(unparseable(&file.path, "the file contains syntax errors"));
        }

        let root = tree.root_node();
        let mut cursor = root.walk();
        let mut declarations = Vec::new();
        for node in root.named_children(&mut cursor) {
            let kind = match node.kind() {
                "function_item" => ElementKind::Function,
                "struct_item" | "enum_item" | "trait_item" | "union_item" | "type_item" => {
                    ElementKind::Type
                }
                "mod_item" => ElementKind::Module,
                _ => continue,
            };
            let Some(name_node) = node.child_by_field_name("name") else {
                continue;
            };
            let name = name_node
                .utf8_text(text.as_bytes())
                .expect("node ranges lie within the parsed text");
            declarations.push(Declaration {
                name: ElementName::new(name).expect("a parsed identifier is never empty"),
                kind,
            });
        }
        Ok(declarations)
    }
}

fn unparseable(path: &SourcePath, reason: &str) -> SyntaxAnalysisError {
    SyntaxAnalysisError::Unparseable {
        path: path.clone(),
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust_file(contents: &str) -> SourceFile {
        SourceFile {
            path: SourcePath::new("src/lib.rs").unwrap(),
            contents: contents.into(),
        }
    }

    fn declared_names(contents: &str) -> Vec<(String, ElementKind)> {
        RustSyntaxAnalyzer
            .analyze(&rust_file(contents))
            .unwrap()
            .into_iter()
            .map(|d| (d.name.to_string(), d.kind))
            .collect()
    }

    #[test]
    fn only_rust_files_are_supported() {
        assert!(RustSyntaxAnalyzer.supports(&SourcePath::new("src/lib.rs").unwrap()));
        assert!(!RustSyntaxAnalyzer.supports(&SourcePath::new("README.md").unwrap()));
    }

    #[test]
    fn top_level_functions_types_and_modules_are_declared() {
        let names = declared_names(
            "pub fn connect() {}\n\
             pub struct Session;\n\
             pub enum State { Open }\n\
             pub trait Close {}\n\
             mod internal {}\n",
        );
        assert_eq!(
            names,
            [
                ("connect".to_owned(), ElementKind::Function),
                ("Session".to_owned(), ElementKind::Type),
                ("State".to_owned(), ElementKind::Type),
                ("Close".to_owned(), ElementKind::Type),
                ("internal".to_owned(), ElementKind::Module),
            ]
        );
    }

    #[test]
    fn nested_items_are_not_declared() {
        let names = declared_names("mod outer { pub fn inner() {} }\n");
        assert_eq!(names, [("outer".to_owned(), ElementKind::Module)]);
    }

    #[test]
    fn a_file_with_syntax_errors_is_rejected() {
        let result = RustSyntaxAnalyzer.analyze(&rust_file("pub fn broken( {\n"));
        assert!(matches!(
            result,
            Err(SyntaxAnalysisError::Unparseable { .. })
        ));
    }

    #[test]
    fn non_utf8_contents_are_rejected() {
        let file = SourceFile {
            path: SourcePath::new("src/lib.rs").unwrap(),
            contents: vec![0xff, 0xfe],
        };
        assert!(matches!(
            RustSyntaxAnalyzer.analyze(&file),
            Err(SyntaxAnalysisError::NonUtf8Text { .. })
        ));
    }
}
