//! The arguments of each tool. Their documentation becomes the JSON Schema
//! that agents read, so it is written for them.

use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextParams {
    /// The source file, relative to the configuration directory or absolute.
    /// It need not exist yet: give the path a new file will have.
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckCodeParams {
    /// Where the code will be, relative to the configuration directory or
    /// absolute. The rules and the dictionary are chosen by this path.
    pub path: String,
    /// The whole contents of the file, not a fragment of it.
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckFilesParams {
    /// Files or directories to check, relative to the configuration
    /// directory or absolute. Leave it out to check the whole project.
    #[serde(default)]
    pub paths: Vec<String>,
    /// The most problems to list. Defaults to 100.
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateDictionaryParams {
    /// Where to create `terms.yaml`, relative to the configuration directory
    /// or absolute, within the project. Its parent directory must exist.
    /// An existing path is never overwritten.
    pub dictionary: String,
    /// The non-empty name of the new bounded context. It is never inferred
    /// from the path.
    pub context: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddTermParams {
    /// The dictionary to edit: an existing `terms.yaml`, relative to the
    /// configuration directory or absolute.
    pub dictionary: String,
    /// The canonical spelling of the term, in ASCII letters and digits
    /// starting with a letter, such as `LineItem`. Its singular form is the
    /// words of the name (`line item`).
    pub name: String,
    /// The plural form, in lowercase words separated by single spaces,
    /// such as `line items`.
    pub plural: String,
    /// The kind of term, such as `entity` or `value_object`.
    pub kind: Option<String>,
    /// A Japanese label, such as `注文`.
    pub ja: Option<String>,
    /// What the term means in the domain.
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditTermParams {
    /// The dictionary to edit: an existing `terms.yaml`, relative to the
    /// configuration directory or absolute.
    pub dictionary: String,
    /// The name of the term to edit, exactly as in the dictionary.
    pub name: String,
    /// A new name. The singular form follows it; give `plural` too if the
    /// singular form changes.
    pub new_name: Option<String>,
    /// A new plural form, in lowercase words.
    pub plural: Option<String>,
    /// A new kind of term.
    pub kind: Option<String>,
    /// Remove the kind of term.
    #[serde(default)]
    pub unset_kind: bool,
    /// A new Japanese label.
    pub ja: Option<String>,
    /// Remove the Japanese label.
    #[serde(default)]
    pub unset_ja: bool,
    /// A new description.
    pub description: Option<String>,
    /// Remove the description.
    #[serde(default)]
    pub unset_description: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveTermParams {
    /// The dictionary to edit: an existing `terms.yaml`, relative to the
    /// configuration directory or absolute.
    pub dictionary: String,
    /// The name of the term to remove, exactly as in the dictionary.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddBehaviorParams {
    /// The dictionary to edit: an existing `terms.yaml`, relative to the
    /// configuration directory or absolute.
    pub dictionary: String,
    /// The name of the term, exactly as in the dictionary.
    pub term: String,
    /// The verb in its base form, in lowercase words, such as `place`.
    pub verb: String,
    /// The third-person singular present, such as `places`.
    pub s: String,
    /// The past tense, such as `placed`.
    pub past: String,
    /// The past participle, such as `placed`.
    pub pp: String,
    /// The present participle, such as `placing`.
    pub ing: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditBehaviorParams {
    /// The dictionary to edit: an existing `terms.yaml`, relative to the
    /// configuration directory or absolute.
    pub dictionary: String,
    /// The name of the term, exactly as in the dictionary.
    pub term: String,
    /// The verb of the behavior to edit, in lowercase words.
    pub verb: String,
    /// A new verb. The base form follows it; give `s`, `past`, `pp` and
    /// `ing` too.
    pub new_verb: Option<String>,
    /// A new third-person singular present.
    pub s: Option<String>,
    /// A new past tense.
    pub past: Option<String>,
    /// A new past participle.
    pub pp: Option<String>,
    /// A new present participle.
    pub ing: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveBehaviorParams {
    /// The dictionary to edit: an existing `terms.yaml`, relative to the
    /// configuration directory or absolute.
    pub dictionary: String,
    /// The name of the term, exactly as in the dictionary.
    pub term: String,
    /// The verb of the behavior to remove, in lowercase words.
    pub verb: String,
}
