//! Editing dictionaries through commands instead of by hand.
//!
//! An [`Edit`] adds, changes or removes one term or one behavior. [`apply`]
//! checks it against the current dictionary, rewrites only the lines it
//! affects so that comments and formatting elsewhere stay as they are, and
//! reads the result back to make sure it is exactly the intended dictionary.
//! Nothing is written to disk here; the caller decides where the text goes.
//!
//! An edit is refused, leaving the source untouched, when the dictionary is
//! already invalid, when the edit would break a rule of the schema (such as
//! a duplicate term or a noun form used by two terms), or when the part of
//! the file to change is laid out in a way that cannot be edited in place.

mod layout;

use crate::diagnostic::Diagnostic;
use crate::dictionary::{self, Behavior, Dictionary, NounForms, Term, VerbForms};
use crate::location::Location;
use crate::words::Words;
use crate::yaml::quoted_list;

use layout::{Item, Step};

/// A change to a dictionary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Adds a term, without behaviors, after the existing terms.
    AddTerm(NewTerm),
    /// Changes fields of the term named `name`.
    EditTerm { name: String, changes: TermChanges },
    /// Removes the term named `name`, together with its behaviors.
    RemoveTerm { name: String },
    /// Adds a behavior to the term named `term`, after its existing
    /// behaviors. The verb is `forms.base`.
    AddBehavior { term: String, forms: VerbForms },
    /// Changes the behavior `verb` of the term named `term`.
    EditBehavior {
        term: String,
        verb: Words,
        changes: BehaviorChanges,
    },
    /// Removes the behavior `verb` from the term named `term`.
    RemoveBehavior { term: String, verb: Words },
}

/// A term to add. Its singular form is the words of its name, as the schema
/// requires, so only the plural form is given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewTerm {
    pub name: String,
    pub plural: Words,
    pub kind: Option<String>,
    pub ja: Option<String>,
    pub description: Option<String>,
}

/// Changes to the fields of a term. Fields left as they are by default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TermChanges {
    /// A new name. The singular form follows it; if that changes the
    /// singular form, `plural` must be given too.
    pub name: Option<String>,
    pub plural: Option<Words>,
    pub kind: FieldChange,
    pub ja: FieldChange,
    pub description: FieldChange,
}

/// A change to an optional text field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FieldChange {
    #[default]
    Keep,
    Set(String),
    /// Removes the field.
    Unset,
}

/// Changes to a behavior. Fields left as they are by default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BehaviorChanges {
    /// A new verb. The base form follows it, and the other forms must all
    /// be given too.
    pub verb: Option<Words>,
    pub s: Option<Words>,
    pub past: Option<Words>,
    pub pp: Option<Words>,
    pub ing: Option<Words>,
}

/// The result of a successful edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The dictionary already had the requested values.
    Unchanged,
    /// The new source of the dictionary.
    Changed(String),
}

/// Why an edit was refused. The source is never partially edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    /// The dictionary is invalid as it is, so it cannot be edited safely.
    InvalidDictionary(Vec<Diagnostic>),
    /// The edit contradicts the dictionary or its schema. Every problem
    /// found is listed.
    Rejected(Vec<String>),
    /// The part of the file to change is laid out in a way that cannot be
    /// edited without rewriting it.
    Layout(String),
}

/// Creates the text of an empty dictionary for the explicitly named context.
/// The context is escaped as YAML and validated before any file is written.
pub fn create(context: &str) -> Result<String, Vec<Diagnostic>> {
    let source = format!("context: {}\nterms: []\n", layout::scalar(context));
    Dictionary::from_yaml_str(&source)?;
    Ok(source)
}

/// Applies `edit` to `source`, the text of a dictionary.
pub fn apply(source: &str, edit: &Edit) -> Result<Outcome, EditError> {
    let current = Dictionary::from_yaml_str(source).map_err(EditError::InvalidDictionary)?;
    let (expected, steps) = plan(&current, edit).map_err(EditError::Rejected)?;
    if same_definitions(&expected, &current) {
        return Ok(Outcome::Unchanged);
    }
    let mut text = source.to_owned();
    for step in &steps {
        text = layout::apply(&text, step).map_err(|reason| {
            EditError::Layout(format!("cannot edit the dictionary in place: {reason}"))
        })?;
    }
    // The steps only see the YAML tree, so check that the text reads back
    // as exactly the intended dictionary before anyone writes it.
    match Dictionary::from_yaml_str(&text) {
        Ok(edited) if same_definitions(&edited, &expected) => Ok(Outcome::Changed(text)),
        Ok(_) => Err(EditError::Layout(
            "cannot edit the dictionary in place: the edited file would not read back as \
             the intended dictionary"
                .to_owned(),
        )),
        Err(diagnostics) => Err(EditError::Layout(format!(
            "cannot edit the dictionary in place: the edited file would be invalid ({})",
            diagnostics
                .first()
                .map_or_else(String::new, ToString::to_string)
        ))),
    }
}

/// Works out the dictionary `edit` should produce and the steps that turn
/// the source into it, or every reason the edit is refused.
fn plan(current: &Dictionary, edit: &Edit) -> Result<(Dictionary, Vec<Step>), Vec<String>> {
    let mut problems = Vec::new();
    let mut expected = current.clone();
    let steps = match edit {
        Edit::AddTerm(new) => {
            // The singular form comes from the name, so only a valid name
            // has one to check.
            let singular = check_name(&mut problems, current, &new.name, None)
                .then(|| dictionary::singular_of(&new.name));
            match &singular {
                Some(Ok(singular)) => {
                    check_forms(&mut problems, current, None, [singular, &new.plural]);
                }
                Some(Err(message)) => problems.push(message.clone()),
                None => check_forms(&mut problems, current, None, [&new.plural; 2]),
            }
            for (field, value) in [
                ("kind", &new.kind),
                ("ja", &new.ja),
                ("description", &new.description),
            ] {
                if let Some(value) = value {
                    check_text(&mut problems, field, value);
                }
            }
            if !problems.is_empty() {
                return Err(problems);
            }
            expected.terms.push(Term {
                name: new.name.clone(),
                kind: new.kind.clone(),
                ja: new.ja.clone(),
                description: new.description.clone(),
                forms: NounForms {
                    singular: singular
                        .and_then(Result::ok)
                        .expect("a valid name has a singular form"),
                    plural: new.plural.clone(),
                },
                behaviors: Vec::new(),
                location: NOWHERE,
            });
            vec![Step::AppendTerm(new.clone())]
        }
        Edit::EditTerm { name, changes } => {
            let index = find_term(current, name).map_err(|message| vec![message])?;
            if *changes == TermChanges::default() {
                return Err(vec!["no changes were given".to_owned()]);
            }
            let old = &current.terms[index];
            let mut new = old.clone();
            if let Some(name) = &changes.name {
                if check_name(&mut problems, current, name, Some(index)) {
                    match dictionary::singular_of(name) {
                        Ok(singular) => {
                            if singular != old.forms.singular && changes.plural.is_none() {
                                problems.push(format!(
                                    "renaming `{}` to `{name}` changes its singular form to \
                                     `{singular}`; give its plural form as well",
                                    old.name
                                ));
                            }
                            new.forms.singular = singular;
                        }
                        Err(message) => problems.push(message),
                    }
                }
                new.name = name.clone();
            }
            if let Some(plural) = &changes.plural {
                new.forms.plural = plural.clone();
            }
            check_forms(
                &mut problems,
                current,
                Some(index),
                [&new.forms.singular, &new.forms.plural],
            );
            for (field, change, value) in [
                ("kind", &changes.kind, &mut new.kind),
                ("ja", &changes.ja, &mut new.ja),
                ("description", &changes.description, &mut new.description),
            ] {
                match change {
                    FieldChange::Keep => {}
                    FieldChange::Set(text) => {
                        check_text(&mut problems, field, text);
                        *value = Some(text.clone());
                    }
                    FieldChange::Unset => *value = None,
                }
            }
            if !problems.is_empty() {
                return Err(problems);
            }
            let steps = term_steps(index, old, &new);
            expected.terms[index] = new;
            steps
        }
        Edit::RemoveTerm { name } => {
            let index = find_term(current, name).map_err(|message| vec![message])?;
            expected.terms.remove(index);
            vec![Step::RemoveTerm(index)]
        }
        Edit::AddBehavior { term, forms } => {
            let index = find_term(current, term).map_err(|message| vec![message])?;
            let owner = &current.terms[index];
            if let Some(existing) = owner.behaviors.iter().find(|b| b.verb == forms.base) {
                return Err(vec![format!(
                    "behavior `{}` is already defined on term `{}` (line {})",
                    forms.base, owner.name, existing.location.line
                )]);
            }
            expected.terms[index].behaviors.push(Behavior {
                verb: forms.base.clone(),
                forms: forms.clone(),
                location: NOWHERE,
            });
            vec![Step::AppendBehavior {
                term: index,
                forms: forms.clone(),
            }]
        }
        Edit::EditBehavior {
            term,
            verb,
            changes,
        } => {
            let index = find_term(current, term).map_err(|message| vec![message])?;
            let owner = &current.terms[index];
            let behavior = find_behavior(owner, verb).map_err(|message| vec![message])?;
            if *changes == BehaviorChanges::default() {
                return Err(vec!["no changes were given".to_owned()]);
            }
            let old = &owner.behaviors[behavior];
            let mut new = old.clone();
            if let Some(verb) = &changes.verb {
                if let Some(existing) = owner
                    .behaviors
                    .iter()
                    .enumerate()
                    .find(|(other, b)| *other != behavior && b.verb == *verb)
                    .map(|(_, b)| b)
                {
                    problems.push(format!(
                        "behavior `{verb}` is already defined on term `{}` (line {})",
                        owner.name, existing.location.line
                    ));
                }
                if *verb != old.verb {
                    let missing: Vec<&str> = [
                        ("s", &changes.s),
                        ("past", &changes.past),
                        ("pp", &changes.pp),
                        ("ing", &changes.ing),
                    ]
                    .into_iter()
                    .filter(|(_, form)| form.is_none())
                    .map(|(name, _)| name)
                    .collect();
                    if !missing.is_empty() {
                        problems.push(format!(
                            "changing the verb `{}` to `{verb}` changes every form; give {} \
                             as well",
                            old.verb,
                            quoted_list(&missing)
                        ));
                    }
                }
                new.verb = verb.clone();
                new.forms.base = verb.clone();
            }
            for (change, form) in [
                (&changes.s, &mut new.forms.s),
                (&changes.past, &mut new.forms.past),
                (&changes.pp, &mut new.forms.pp),
                (&changes.ing, &mut new.forms.ing),
            ] {
                if let Some(words) = change {
                    *form = words.clone();
                }
            }
            if !problems.is_empty() {
                return Err(problems);
            }
            let steps = behavior_steps(index, behavior, old, &new);
            expected.terms[index].behaviors[behavior] = new;
            steps
        }
        Edit::RemoveBehavior { term, verb } => {
            let index = find_term(current, term).map_err(|message| vec![message])?;
            let behavior =
                find_behavior(&current.terms[index], verb).map_err(|message| vec![message])?;
            expected.terms[index].behaviors.remove(behavior);
            vec![Step::RemoveBehavior {
                term: index,
                behavior,
            }]
        }
    };
    Ok((expected, steps))
}

/// The location given to definitions that are not read from a file yet.
const NOWHERE: Location = Location { line: 0, column: 0 };

/// Finds a term by its exact name.
fn find_term(dictionary: &Dictionary, name: &str) -> Result<usize, String> {
    dictionary
        .terms
        .iter()
        .position(|term| term.name == name)
        .ok_or_else(|| {
            let names: Vec<&str> = dictionary.terms.iter().map(|t| t.name.as_str()).collect();
            if names.is_empty() {
                format!("no term `{name}` in the dictionary (no terms are defined)")
            } else {
                format!(
                    "no term `{name}` in the dictionary (defined terms: {})",
                    quoted_list(&names)
                )
            }
        })
}

/// Finds a behavior of `term` by its verb.
fn find_behavior(term: &Term, verb: &Words) -> Result<usize, String> {
    term.behaviors
        .iter()
        .position(|behavior| behavior.verb == *verb)
        .ok_or_else(|| {
            let verbs: Vec<String> = term.behaviors.iter().map(|b| b.verb.to_string()).collect();
            let verbs: Vec<&str> = verbs.iter().map(String::as_str).collect();
            if verbs.is_empty() {
                format!(
                    "no behavior `{verb}` on term `{}` (no behaviors are defined)",
                    term.name
                )
            } else {
                format!(
                    "no behavior `{verb}` on term `{}` (defined behaviors: {})",
                    term.name,
                    quoted_list(&verbs)
                )
            }
        })
}

/// Checks that `name` is a valid term name that no other term has,
/// ignoring the term at `except`. Returns whether the name is valid.
fn check_name(
    problems: &mut Vec<String>,
    dictionary: &Dictionary,
    name: &str,
    except: Option<usize>,
) -> bool {
    if let Err(message) = dictionary::validate_term_name(name) {
        problems.push(message);
        return false;
    }
    let clash = dictionary
        .terms
        .iter()
        .enumerate()
        .find(|(index, term)| Some(*index) != except && term.name.eq_ignore_ascii_case(name));
    if let Some((_, term)) = clash {
        problems.push(if term.name == name {
            format!(
                "term `{name}` is already defined (line {})",
                term.location.line
            )
        } else {
            format!(
                "term `{name}` clashes with term `{}` (line {}): term names are compared \
                 ignoring case",
                term.name, term.location.line
            )
        });
    }
    true
}

/// Checks that no term other than the one at `except` uses `forms`.
fn check_forms(
    problems: &mut Vec<String>,
    dictionary: &Dictionary,
    except: Option<usize>,
    forms: [&Words; 2],
) {
    let [singular, plural] = forms;
    let own: &[&Words] = if singular == plural {
        &[singular]
    } else {
        &[singular, plural]
    };
    for form in own {
        let user = dictionary
            .terms
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != except)
            .map(|(_, term)| term)
            .find(|term| term.forms.singular == **form || term.forms.plural == **form);
        if let Some(term) = user {
            problems.push(format!(
                "the form `{form}` is already used by term `{}` (line {})",
                term.name, term.location.line
            ));
        }
    }
}

/// Checks a text field the same way the dictionary reader does.
fn check_text(problems: &mut Vec<String>, field: &str, text: &str) {
    if text.trim().is_empty() {
        problems.push(format!("`{field}` must not be empty"));
    }
}

/// Returns the steps that turn the term `old`, at `index`, into `new`,
/// touching only the fields that differ.
fn term_steps(index: usize, old: &Term, new: &Term) -> Vec<Step> {
    let item = Item::Term(index);
    let mut steps = Vec::new();
    if new.name != old.name {
        steps.push(Step::Set {
            item,
            path: &["name"],
            value: new.name.clone(),
        });
    }
    for (path, old_form, new_form) in [
        (
            &["forms", "singular"],
            &old.forms.singular,
            &new.forms.singular,
        ),
        (&["forms", "plural"], &old.forms.plural, &new.forms.plural),
    ] {
        if new_form != old_form {
            steps.push(Step::Set {
                item,
                path,
                value: new_form.to_string(),
            });
        }
    }
    let texts: [(&'static [&'static str], &Option<String>, &Option<String>); 3] = [
        (&["kind"], &old.kind, &new.kind),
        (&["ja"], &old.ja, &new.ja),
        (&["description"], &old.description, &new.description),
    ];
    for (path, old_value, new_value) in texts {
        match (old_value, new_value) {
            (old_value, Some(value)) if old_value.as_ref() != Some(value) => {
                steps.push(Step::Set {
                    item,
                    path,
                    value: value.clone(),
                });
            }
            (Some(_), None) => steps.push(Step::Unset {
                term: index,
                key: path[0],
            }),
            _ => {}
        }
    }
    steps
}

/// Returns the steps that turn the behavior `old` into `new`, touching only
/// the fields that differ.
fn behavior_steps(term: usize, behavior: usize, old: &Behavior, new: &Behavior) -> Vec<Step> {
    let item = Item::Behavior(term, behavior);
    let mut steps = Vec::new();
    if new.verb != old.verb {
        steps.push(Step::Set {
            item,
            path: &["verb"],
            value: new.verb.to_string(),
        });
    }
    let paths: [&'static [&'static str]; 5] = [
        &["forms", "base"],
        &["forms", "s"],
        &["forms", "past"],
        &["forms", "pp"],
        &["forms", "ing"],
    ];
    for ((path, old_form), new_form) in paths.into_iter().zip(old.forms.all()).zip(new.forms.all())
    {
        if new_form != old_form {
            steps.push(Step::Set {
                item,
                path,
                value: new_form.to_string(),
            });
        }
    }
    steps
}

/// Tells whether two dictionaries define the same things, wherever in a
/// file they are defined.
fn same_definitions(a: &Dictionary, b: &Dictionary) -> bool {
    without_locations(a) == without_locations(b)
}

fn without_locations(dictionary: &Dictionary) -> Dictionary {
    let mut dictionary = dictionary.clone();
    for term in &mut dictionary.terms {
        term.location = NOWHERE;
        for behavior in &mut term.behaviors {
            behavior.location = NOWHERE;
        }
    }
    dictionary
}
