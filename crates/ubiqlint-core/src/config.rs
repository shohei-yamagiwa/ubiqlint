//! Configuration (`ubiqlint.yaml`): which naming patterns apply to which
//! identifiers, and where.

use std::collections::HashMap;
use std::path::Path;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::diagnostic::{Diagnostic, Location};
use crate::pattern::{Pattern, RESERVED_NAMES};
use crate::words::Words;
use crate::yaml::{self, Diagnostics, Fields, Node};

/// The file name of a configuration file.
pub const CONFIG_FILE_NAME: &str = "ubiqlint.yaml";

/// The contents of a configuration file.
#[derive(Clone, Debug)]
pub struct Config {
    pub vocabularies: Vec<Vocabulary>,
    pub rules: Vec<Rule>,
}

/// A named list of word sequences that patterns can refer to, such as
/// `role: [repository, factory]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vocabulary {
    pub name: String,
    pub words: Vec<Words>,
    pub location: Location,
}

/// The patterns allowed for one kind of identifier in a set of files.
#[derive(Clone, Debug)]
pub struct Rule {
    pub id: String,
    /// Glob patterns for the files the rule applies to, as written.
    pub include: Vec<String>,
    pub target: Target,
    /// The identifier must match at least one of these.
    pub patterns: Vec<Pattern>,
    pub location: Location,
    include_set: GlobSet,
}

/// The kind of identifier a rule applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// Classes, interfaces, enums, records and similar declarations.
    Type,
    Method,
    Field,
    Parameter,
    /// Local variables.
    Variable,
}

const TARGETS: [(&str, Target); 5] = [
    ("type", Target::Type),
    ("method", Target::Method),
    ("field", Target::Field),
    ("parameter", Target::Parameter),
    ("variable", Target::Variable),
];

impl Target {
    /// Returns the name used in configuration files.
    pub fn name(self) -> &'static str {
        TARGETS
            .iter()
            .find(|(_, target)| *target == self)
            .map(|(name, _)| *name)
            .expect("every target has a name")
    }
}

impl Config {
    /// Reads a configuration from YAML, reporting every problem found.
    pub fn from_yaml_str(source: &str) -> Result<Self, Vec<Diagnostic>> {
        let root = yaml::parse_document(source)?;
        let mut diags = Diagnostics::default();
        let config = read_config(&mut diags, &root);
        diags.finish(config)
    }
}

impl Rule {
    /// Tells whether the rule applies to `path`, given relative to the
    /// directory of the configuration file.
    pub fn includes(&self, path: &Path) -> bool {
        self.include_set.is_match(path)
    }
}

fn read_config(diags: &mut Diagnostics, node: &Node) -> Option<Config> {
    let fields = Fields::new(diags, node, "the configuration", &["vocabularies", "rules"])?;
    // Patterns may refer to every vocabulary that is declared, even an
    // invalid one, so that a bad vocabulary is reported once instead of
    // again at each pattern that uses it.
    let mut names = Vec::new();
    let vocabularies = yaml::optional(&fields, "vocabularies", |node| {
        read_vocabularies(diags, node, &mut names)
    });
    let rules = fields
        .required(diags, "rules")
        .and_then(|node| yaml::sequence(diags, node, "`rules`"))
        .map(|items| {
            items
                .iter()
                .filter_map(|item| read_rule(diags, item, &names))
                .collect::<Vec<_>>()
        });
    if let Some(rules) = &rules {
        check_unique_rules(diags, rules);
    }
    Some(Config {
        vocabularies: vocabularies?.unwrap_or_default(),
        rules: rules?,
    })
}

/// Reads the vocabularies, adding the name of every declared vocabulary to
/// `names`.
fn read_vocabularies(
    diags: &mut Diagnostics,
    node: &Node,
    names: &mut Vec<String>,
) -> Option<Vec<Vocabulary>> {
    let entries = yaml::entries(diags, node, "`vocabularies`")?;
    let mut vocabularies = Vec::with_capacity(entries.len());
    let mut failed = false;
    for (name, key, value) in entries {
        names.push(name.to_owned());
        let valid_name = is_snake_case(name) && !RESERVED_NAMES.contains(&name);
        if !valid_name {
            let reason = if RESERVED_NAMES.contains(&name) {
                format!("`{name}` is reserved for a placeholder")
            } else {
                "use lowercase letters, digits and `_`, starting with a letter".to_owned()
            };
            diags.error(
                yaml::location(key),
                format!("invalid vocabulary name `{name}`: {reason}"),
            );
        }
        let what = format!("vocabulary `{name}`");
        let words = yaml::non_empty_sequence(diags, value, &what).map(|items| {
            yaml::all(
                items
                    .iter()
                    .map(|item| yaml::words(diags, item, &format!("entry in {what}"))),
            )
        });
        match (valid_name, words.flatten()) {
            (true, Some(words)) => vocabularies.push(Vocabulary {
                name: name.to_owned(),
                words,
                location: yaml::location(key),
            }),
            _ => failed = true,
        }
    }
    (!failed).then_some(vocabularies)
}

fn read_rule(diags: &mut Diagnostics, node: &Node, vocabularies: &[String]) -> Option<Rule> {
    let fields = Fields::new(
        diags,
        node,
        "a rule",
        &["id", "include", "target", "patterns"],
    )?;
    let id = fields
        .required(diags, "id")
        .and_then(|node| read_rule_id(diags, node));
    let include = fields
        .required(diags, "include")
        .and_then(|node| read_include(diags, node));
    let target = fields
        .required(diags, "target")
        .and_then(|node| read_target(diags, node));
    let patterns = fields.required(diags, "patterns").and_then(|node| {
        let items = yaml::non_empty_sequence(diags, node, "`patterns`")?;
        yaml::all(items.iter().map(|item| {
            let source = yaml::string(diags, item, "a pattern")?;
            Pattern::parse(source, |name| vocabularies.iter().any(|v| v == name))
                .map_err(|message| diags.error(yaml::location(item), message))
                .ok()
        }))
    });
    let (include, include_set) = include?;
    Some(Rule {
        id: id?.to_owned(),
        include,
        target: target?,
        patterns: patterns?,
        location: yaml::location(node),
        include_set,
    })
}

fn read_rule_id<'a>(diags: &mut Diagnostics, node: &'a Node) -> Option<&'a str> {
    let id = yaml::string(diags, node, "`id`")?;
    let valid = !id.is_empty()
        && id.split('-').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        });
    if !valid {
        diags.error(
            yaml::location(node),
            format!(
                "invalid rule id `{id}`: use lowercase letters and digits, \
                 with single hyphens between words (e.g. `domain-method`)"
            ),
        );
        return None;
    }
    Some(id)
}

fn read_include(diags: &mut Diagnostics, node: &Node) -> Option<(Vec<String>, GlobSet)> {
    let items = yaml::non_empty_sequence(diags, node, "`include`")?;
    let mut globs = Vec::with_capacity(items.len());
    let mut builder = GlobSetBuilder::new();
    let mut failed = false;
    for item in items {
        let Some(text) = yaml::non_empty_string(diags, item, "a glob in `include`") else {
            failed = true;
            continue;
        };
        match GlobBuilder::new(text).literal_separator(true).build() {
            Ok(glob) => {
                builder.add(glob);
                globs.push(text.to_owned());
            }
            Err(error) => {
                diags.error(
                    yaml::location(item),
                    format!("invalid glob `{text}`: {}", error.kind()),
                );
                failed = true;
            }
        }
    }
    if failed {
        return None;
    }
    match builder.build() {
        Ok(set) => Some((globs, set)),
        Err(error) => {
            diags.error(
                yaml::location(node),
                format!("invalid globs in `include`: {error}"),
            );
            None
        }
    }
}

fn read_target(diags: &mut Diagnostics, node: &Node) -> Option<Target> {
    let name = yaml::string(diags, node, "`target`")?;
    let target = TARGETS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, target)| *target);
    if target.is_none() {
        let expected: Vec<&str> = TARGETS.iter().map(|(candidate, _)| *candidate).collect();
        diags.error(
            yaml::location(node),
            format!(
                "unknown target `{name}`, expected one of {}",
                yaml::quoted_list(&expected)
            ),
        );
    }
    target
}

fn is_snake_case(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Reports rules whose id is already used by an earlier rule.
fn check_unique_rules(diags: &mut Diagnostics, rules: &[Rule]) {
    let mut ids: HashMap<&str, &Rule> = HashMap::new();
    for rule in rules {
        match ids.get(rule.id.as_str()) {
            Some(first) => diags.error(
                rule.location,
                format!(
                    "duplicate rule id `{}` (first used at line {})",
                    rule.id, first.location.line
                ),
            ),
            None => {
                ids.insert(&rule.id, rule);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::{Element, NounForm, VerbForm};

    /// Formats the diagnostics for `source` as `line:column: message`.
    fn errors(source: &str) -> Vec<String> {
        Config::from_yaml_str(source)
            .expect_err("the configuration should be invalid")
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    const EXAMPLE: &str = r#"
vocabularies:
  role: [repository, factory, policy]

rules:
  - id: domain-type
    include: ["src/main/java/**/domain/**"]
    target: type
    patterns:
      - "{noun}"
      - "{noun}{role}"
      - "{noun}{verb:pp}"

  - id: domain-method
    include: ["src/main/java/**/domain/**"]
    target: method
    patterns:
      - "{verb}"
      - "{verb}{noun}"

  - id: dto
    include: ["src/main/java/**/dto/**"]
    target: method
    patterns:
      - "get{any}"
      - "set{any}"
"#;

    #[test]
    fn reads_a_valid_configuration() {
        let config = Config::from_yaml_str(EXAMPLE).unwrap();

        assert_eq!(config.vocabularies.len(), 1);
        let role = &config.vocabularies[0];
        assert_eq!(role.name, "role");
        let words: Vec<String> = role.words.iter().map(ToString::to_string).collect();
        assert_eq!(words, ["repository", "factory", "policy"]);

        let ids: Vec<&str> = config.rules.iter().map(|rule| rule.id.as_str()).collect();
        assert_eq!(ids, ["domain-type", "domain-method", "dto"]);

        let rule = &config.rules[0];
        assert_eq!(rule.target, Target::Type);
        assert_eq!(rule.include, ["src/main/java/**/domain/**"]);
        assert_eq!(rule.location, Location { line: 6, column: 5 });
        assert_eq!(
            rule.patterns[2].elements(),
            [
                Element::Noun(NounForm::Singular),
                Element::Verb(VerbForm::Pp)
            ]
        );
        assert_eq!(
            rule.patterns[1].elements(),
            [
                Element::Noun(NounForm::Singular),
                Element::Vocabulary("role".to_owned())
            ]
        );
    }

    #[test]
    fn vocabularies_are_optional() {
        let source = r#"
rules:
  - id: domain-method
    include: ["**/domain/**"]
    target: method
    patterns: ["{verb}"]
"#;
        let config = Config::from_yaml_str(source).unwrap();
        assert!(config.vocabularies.is_empty());
    }

    #[test]
    fn matches_paths_against_include_globs() {
        let config = Config::from_yaml_str(EXAMPLE).unwrap();
        let domain = &config.rules[0];
        assert!(domain.includes(Path::new(
            "src/main/java/com/example/order/domain/Order.java"
        )));
        assert!(!domain.includes(Path::new(
            "src/main/java/com/example/order/dto/OrderDto.java"
        )));
        assert!(!domain.includes(Path::new("src/test/java/domain/OrderTest.java")));
    }

    #[test]
    fn single_star_does_not_cross_directories() {
        let source = r#"
rules:
  - id: top-level
    include: ["src/*.java"]
    target: type
    patterns: ["{noun}"]
"#;
        let config = Config::from_yaml_str(source).unwrap();
        assert!(config.rules[0].includes(Path::new("src/Order.java")));
        assert!(!config.rules[0].includes(Path::new("src/order/Order.java")));
    }

    #[test]
    fn reports_problems_in_rules() {
        let source = r#"
rules:
  - id: Domain_Method
    include: []
    target: class
    patterns: ["{verb}{entity}"]
  - include: ["src/[a"]
    target: method
    patterns: []
"#;
        assert_eq!(
            errors(source),
            [
                "3:9: invalid rule id `Domain_Method`: use lowercase letters and digits, \
                 with single hyphens between words (e.g. `domain-method`)",
                "4:14: `include` must not be empty",
                "5:13: unknown target `class`, expected one of \
                 `type`, `method`, `field`, `parameter`, `variable`",
                "6:16: unknown placeholder `{entity}`: expected `{noun}`, `{verb}`, `{any}` \
                 or a vocabulary defined in `vocabularies`",
                "7:5: missing required field `id` in a rule",
                "7:15: invalid glob `src/[a`: unclosed character class; missing ']'",
                "9:15: `patterns` must not be empty",
            ]
        );
    }

    #[test]
    fn reports_invalid_vocabularies() {
        let source = r#"
vocabularies:
  noun: [entity]
  Role: [repository]
  suffix: []
  layer: [Domain]
rules: []
"#;
        assert_eq!(
            errors(source),
            [
                "3:3: invalid vocabulary name `noun`: `noun` is reserved for a placeholder",
                "4:3: invalid vocabulary name `Role`: use lowercase letters, digits and `_`, \
                 starting with a letter",
                "5:11: vocabulary `suffix` must not be empty",
                "6:11: invalid entry in vocabulary `layer`: `Domain` is not a valid word \
                 sequence: use lowercase letters and digits, with a single space between words \
                 (e.g. `line item`)",
            ]
        );
    }

    #[test]
    fn reports_every_invalid_word_and_keeps_vocabulary_names() {
        let source = r#"
vocabularies:
  role: [Repository, Factory]
rules:
  - id: domain-type
    include: ["**/domain/**"]
    target: type
    patterns: ["{noun}{role}"]
"#;
        assert_eq!(
            errors(source),
            [
                "3:10: invalid entry in vocabulary `role`: `Repository` is not a valid word \
                 sequence: use lowercase letters and digits, with a single space between words \
                 (e.g. `line item`)",
                "3:22: invalid entry in vocabulary `role`: `Factory` is not a valid word \
                 sequence: use lowercase letters and digits, with a single space between words \
                 (e.g. `line item`)",
            ]
        );
    }

    #[test]
    fn reports_duplicate_rule_ids_and_unknown_fields() {
        let source = r#"
rules:
  - id: domain
    include: ["**/domain/**"]
    target: type
    patterns: ["{noun}"]
  - id: domain
    include: ["**/domain/**"]
    target: method
    patterns: ["{verb}"]
    exclude: ["**/test/**"]
"#;
        assert_eq!(
            errors(source),
            [
                "7:5: duplicate rule id `domain` (first used at line 3)",
                "11:5: unknown field `exclude` in a rule, expected one of \
                 `id`, `include`, `target`, `patterns`",
            ]
        );
    }

    #[test]
    fn reports_a_missing_rules_field() {
        assert_eq!(
            errors("vocabularies: {}\n"),
            ["1:1: missing required field `rules` in the configuration"]
        );
    }

    #[test]
    fn target_names_round_trip() {
        for (name, target) in TARGETS {
            assert_eq!(target.name(), name);
        }
    }
}
