//! Runs the naming library on real names and folders from the command line. The tool
//! reads a folder as a caller would, collecting its entries and rule files, and passes
//! only names to the library. It never renames anything.

use clap::{Parser, Subcommand, ValueEnum};
use nomenclator::model::EntryKind;
use nomenclator::{
    CallerInputs, IgnoreDialect, IgnoreFile, ImportPlan, Interpretation, Item, LibraryType,
    MatchFacts, OutputProfile, Overlay, ReferenceMetadata, RenamePlan, Rules, SearchQueries,
    Severity, TargetFilesystem, Validation,
};
use serde::Serialize;
use serde_json::Value as Json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod script;

/// The rule-file name that is always read, in gitignore syntax, in addition to the
/// `--rule-file` names. The caller chooses its rule files (X14). The no-media markers come
/// from the rules.
const DEFAULT_RULE_FILE: &str = ".ignore";

#[derive(Parser)]
#[command(name = "nomenclator", version, about)]
struct Arguments {
    #[command(subcommand)]
    command: Command,
    /// The output format. Defaults to `table`.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table, global = true)]
    format: OutputFormat,
    /// An overlay file (YAML or JSON) that changes the built-in vocabularies. Repeat the
    /// option to apply several overlays. Defaults to no overlay.
    #[arg(long = "overlay", value_name = "FILE", global = true)]
    overlays: Vec<PathBuf>,
    /// A language of the library, as a BCP 47 tag that has a language pack, such as `en`
    /// or `de`. Repeat the option for several languages. Words of other languages are not
    /// read. Defaults to all languages.
    #[arg(long = "language", value_name = "TAG", global = true)]
    languages: Vec<String>,
    /// An additional rule-file name to read, such as `.skip` or `.skip:glob`. The file uses
    /// gitignore syntax, or one glob per line with the `:glob` suffix. Repeat the option
    /// for several names. Defaults to none; `.ignore` and the no-media markers of the rules
    /// (`.nomedia` by default) are always read.
    #[arg(long = "rule-file", value_name = "NAME[:DIALECT]", global = true)]
    rule_files: Vec<String>,
    /// A caller inputs file (YAML or JSON) with hints, overrides and series numbering
    /// (T20, X16, X17). Defaults to no inputs.
    #[arg(long = "inputs", value_name = "FILE", global = true)]
    inputs: Option<PathBuf>,
    /// An output profile file (YAML or JSON) with the layout and validation settings
    /// (R1-R15). Defaults to the built-in profile.
    #[arg(long = "profile", value_name = "FILE", global = true)]
    profile: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Parses one name or root-relative path. A trailing `/` marks a folder. In a Movies
    /// library, the name is parsed as a movie name. In a TV library, a file is parsed as
    /// an episode and a folder as a series.
    Parse {
        /// The name or root-relative path to parse.
        name: String,
        /// The library type. Required.
        #[arg(long, value_enum)]
        library: LibraryType,
    },
    /// Reads the release facts of one name: resolution, source, codecs, audio, release
    /// group and the others (X19). A trailing `/` marks a folder.
    Release {
        /// The name to read.
        name: String,
    },
    /// Suggests whether a library folder's names look like a Movies or a TV library, with
    /// the videos as evidence (X21).
    Suggest {
        /// The library folder to read.
        directory: PathBuf,
    },
    /// Interprets a library folder and prints its items and outcomes.
    Interpret {
        /// The library folder to read.
        directory: PathBuf,
        /// The library type. Required.
        #[arg(long, value_enum)]
        library: LibraryType,
    },
    /// Interprets a library folder and prints the provider search queries of each movie,
    /// series and episode, most specific first (P7).
    Queries {
        /// The library folder to read.
        directory: PathBuf,
        /// The library type. Required.
        #[arg(long, value_enum)]
        library: LibraryType,
    },
    /// Proposes renames for a library folder. Prints the proposals and renames nothing.
    Plan {
        /// The library folder to read.
        directory: PathBuf,
        /// The library type. Required.
        #[arg(long, value_enum)]
        library: LibraryType,
        /// The filesystem that the proposed names must be valid on. Defaults to the
        /// profile's target filesystem, which defaults to `portable`.
        #[arg(long, value_enum)]
        target_filesystem: Option<TargetFilesystem>,
        /// The output profile file (YAML or JSON) that wrote the library. The library is
        /// converted from its layout to the `--profile` layout (R18). Defaults to the
        /// built-in profile.
        #[arg(long, value_name = "FILE")]
        from_profile: Option<PathBuf>,
        /// Prints a script for the shell that performs the proposals, instead of the
        /// table. Defaults to no script.
        #[arg(long, value_enum, value_name = "SHELL")]
        script: Option<script::Shell>,
        /// The file that the script which reverses the proposals is written to. Requires
        /// `--script`. Defaults to no reverse script.
        #[arg(long, value_name = "FILE", requires = "script")]
        undo_script: Option<PathBuf>,
    },
    /// Proposes moves of a download folder's files into a library folder (R17). Prints the
    /// proposals, the blocked moves and the files left in place, and moves nothing.
    Import {
        /// The download folder to read.
        download: PathBuf,
        /// The library folder to import into.
        library_directory: PathBuf,
        /// The library type. Required.
        #[arg(long, value_enum)]
        library: LibraryType,
        /// The filesystem that the proposed names must be valid on. Defaults to the
        /// profile's target filesystem, which defaults to `portable`.
        #[arg(long, value_enum)]
        target_filesystem: Option<TargetFilesystem>,
        /// Prints a script for the shell that performs the moves, instead of the table.
        /// Defaults to no script.
        #[arg(long, value_enum, value_name = "SHELL")]
        script: Option<script::Shell>,
        /// The file that the script which moves the files back is written to. Requires
        /// `--script`. Defaults to no reverse script.
        #[arg(long, value_name = "FILE", requires = "script")]
        undo_script: Option<PathBuf>,
    },
    /// Interprets a library folder under two rule sets and prints the facts that differ,
    /// per path. Each `--rules` value is `builtin` or an overlay file, applied after
    /// `--overlay` and `--language`. Exits with status 1 if any fact differs.
    Diff {
        /// The library folder to read.
        directory: PathBuf,
        /// The library type. Required.
        #[arg(long, value_enum)]
        library: LibraryType,
        /// One rule set: `builtin` or an overlay file. Pass the option exactly twice.
        #[arg(
            long = "rules",
            value_name = "builtin|FILE",
            num_args = 1,
            required = true
        )]
        rules: Vec<String>,
    },
    /// Reports the entries of a library folder that do not follow the profile or cannot
    /// be named safely. Exits with status 1 if a diagnostic is at the `--fail-on` severity
    /// or more serious.
    Validate {
        /// The library folder to read.
        directory: PathBuf,
        /// The library type. Required.
        #[arg(long, value_enum)]
        library: LibraryType,
        /// The filesystem that the proposed names must be valid on. Defaults to the
        /// profile's target filesystem, which defaults to `portable`.
        #[arg(long, value_enum)]
        target_filesystem: Option<TargetFilesystem>,
        /// Shows info diagnostics, which report missing optional facts. Defaults to off.
        #[arg(long)]
        all: bool,
        /// The lowest severity that fails the run. Defaults to `error`.
        #[arg(long, value_enum, default_value_t = Severity::Error)]
        fail_on: Severity,
        /// A reference metadata file (YAML or JSON) with movie years and episode lists
        /// (X18). Defaults to no reference.
        #[arg(long, value_name = "FILE")]
        reference: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Table,
    Json,
}

/// A library folder as root-relative paths and rule files, in the form that the library
/// takes.
#[derive(Default)]
struct ScannedLibrary {
    paths: Vec<String>,
    ignore_files: Vec<IgnoreFile>,
}

/// The rule files that a scan reads: the named files with their dialects, and the
/// no-media markers of each rule set in use.
struct RuleFiles<'a> {
    names: Vec<(String, IgnoreDialect)>,
    rules: Vec<&'a Rules>,
}

impl RuleFiles<'_> {
    /// Returns the dialect in which a scan reads the file `name`. Returns `None` if
    /// `name` is not a rule file. A no-media marker is read in gitignore syntax, because
    /// its contents are not used.
    fn dialect(&self, name: &str) -> Option<IgnoreDialect> {
        if let Some((_, dialect)) = self.names.iter().find(|(known, _)| known == name) {
            return Some(*dialect);
        }
        self.rules
            .iter()
            .any(|rules| rules.is_no_media_marker(name))
            .then_some(IgnoreDialect::Gitignore)
    }
}

/// Runs the command. Exits with status 0 if the run succeeds, 1 if validation finds a
/// diagnostic at `--fail-on` or `diff` finds a difference, and 2 if the run fails.
fn main() -> ExitCode {
    let arguments = Arguments::parse();
    match run(arguments) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}

/// Runs the command in `arguments`. Returns `Ok(false)` if validation found diagnostics
/// that fail the run or `diff` found a difference.
fn run(arguments: Arguments) -> Result<bool, String> {
    let format = arguments.format;
    let rules = read_rules(&arguments.overlays, &arguments.languages)?;
    let rule_file_names = rule_file_names(&arguments.rule_files)?;
    let rule_files = RuleFiles {
        names: rule_file_names.clone(),
        rules: vec![&rules],
    };
    let inputs: CallerInputs = read_document(arguments.inputs.as_deref())?;
    let base_profile: OutputProfile = read_document(arguments.profile.as_deref())?;
    check_profile(&rules, &base_profile)?;
    match arguments.command {
        Command::Suggest { directory } => {
            let scanned = scan_library(&directory, &rule_files)?;
            let suggestion = rules.suggest_library_type(&scanned.paths);
            match format {
                OutputFormat::Json => print_json(&to_json(&suggestion)?)?,
                OutputFormat::Table => {
                    let library = match &suggestion.library {
                        nomenclator::model::Finding::Resolved { value, .. } => vocabulary(value),
                        nomenclator::model::Finding::Ambiguous { .. } => "ambiguous".to_owned(),
                        nomenclator::model::Finding::Unknown => "unknown".to_owned(),
                    };
                    println!("suggested library type: {library}");
                    println!(
                        "{} videos with an episode signal, {} other videos",
                        suggestion.episode_videos.len(),
                        suggestion.movie_videos.len()
                    );
                }
            }
            Ok(true)
        }
        Command::Release { name } => {
            let kind = if name.ends_with('/') {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            let result = to_json(rules.parse_release(name.trim_end_matches('/'), kind))?;
            match format {
                OutputFormat::Json => print_json(&result)?,
                OutputFormat::Table => {
                    let mut rows = Vec::new();
                    collect_fact_rows("", &result, &mut rows);
                    print_table(&["FACT", "VALUE", "EVIDENCE"], rows);
                }
            }
            Ok(true)
        }
        Command::Parse { name, library } => {
            let kind = if name.ends_with('/') {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            let result = match (library, kind) {
                (LibraryType::Movies, _) => {
                    to_json(rules.parse_movie_name(name.trim_end_matches('/'), kind))
                }
                (LibraryType::Tv, EntryKind::File) => {
                    to_json(rules.parse_episode_name(&name, kind, None))
                }
                (LibraryType::Tv, EntryKind::Directory) => to_json(rules.parse_series_name(&name)),
            }?;
            match format {
                OutputFormat::Json => print_json(&result)?,
                OutputFormat::Table => {
                    let mut rows = Vec::new();
                    collect_fact_rows("", &result, &mut rows);
                    print_table(&["FACT", "VALUE", "EVIDENCE"], rows);
                }
            }
            Ok(true)
        }
        Command::Interpret { directory, library } => {
            let scanned = scan_library(&directory, &rule_files)?;
            let interpretation =
                rules.interpret_with(library, &scanned.paths, &scanned.ignore_files, &inputs);
            match format {
                OutputFormat::Json => print_json(&to_json(&interpretation)?)?,
                OutputFormat::Table => print_interpretation(&interpretation),
            }
            Ok(true)
        }
        Command::Queries { directory, library } => {
            let scanned = scan_library(&directory, &rule_files)?;
            let interpretation =
                rules.interpret_with(library, &scanned.paths, &scanned.ignore_files, &inputs);
            let results: Vec<(MatchFacts, SearchQueries)> =
                nomenclator::match_facts(&interpretation.items)
                    .into_iter()
                    .map(|facts| {
                        let queries = rules.search_queries(&facts);
                        (facts, queries)
                    })
                    .collect();
            match format {
                OutputFormat::Json => {
                    let items: Vec<Json> = results
                        .iter()
                        .map(|(facts, queries)| {
                            Ok(serde_json::json!({
                                "facts": to_json(facts)?,
                                "queries": to_json(&queries.queries)?,
                            }))
                        })
                        .collect::<Result<_, String>>()?;
                    print_json(&Json::Array(items))?;
                }
                OutputFormat::Table => {
                    let rows = results.iter().flat_map(|(facts, queries)| {
                        let item = match &facts.kind {
                            Some(kind) => vocabulary(kind),
                            None => "-".to_owned(),
                        };
                        let item = format!("{item} {}", facts.title.clone().unwrap_or_default());
                        queries.queries.iter().map(move |query| {
                            let text = match (&query.provider, &query.id) {
                                (Some(provider), Some(id)) => format!("{provider}: {id}"),
                                _ => query.title.clone().unwrap_or_default(),
                            };
                            let year = query.year.map(|year| year.to_string()).unwrap_or_default();
                            vec![item.clone(), vocabulary(query.reason), text, year]
                        })
                    });
                    print_table(&["ITEM", "QUERY", "TEXT", "YEAR"], rows.collect());
                }
            }
            Ok(true)
        }
        Command::Import {
            download,
            library_directory,
            library,
            target_filesystem,
            script,
            undo_script,
        } => {
            let scanned_download = scan_library(&download, &rule_files)?;
            let scanned_library = scan_library(&library_directory, &rule_files)?;
            let download_interpretation = rules.interpret_with(
                library,
                &scanned_download.paths,
                &scanned_download.ignore_files,
                &inputs,
            );
            let library_interpretation = rules.interpret(
                library,
                &scanned_library.paths,
                &scanned_library.ignore_files,
            );
            let plan = rules.plan_import(
                &download_interpretation,
                &library_interpretation,
                &scanned_library.paths,
                &profile(base_profile, target_filesystem),
            );
            if let Some(shell) = script {
                let download = absolute_folder(&download)?;
                let library = absolute_folder(&library_directory)?;
                if let Some(undo) = &undo_script {
                    let text = script::import_undo_script(&plan, shell, &download, &library);
                    write_file(undo, &text)?;
                }
                print!(
                    "{}",
                    script::import_script(&plan, shell, &download, &library)
                );
                return Ok(true);
            }
            match format {
                OutputFormat::Json => print_json(&to_json(&plan)?)?,
                OutputFormat::Table => print_import(&plan),
            }
            Ok(true)
        }
        Command::Plan {
            directory,
            library,
            target_filesystem,
            from_profile,
            script,
            undo_script,
        } => {
            let from_profile: OutputProfile = read_document(from_profile.as_deref())?;
            check_profile(&rules, &from_profile)?;
            let scanned = scan_library(&directory, &rule_files)?;
            let profile = profile(base_profile, target_filesystem);
            let plan = rules
                .plan_conversion(
                    library,
                    &scanned.paths,
                    &scanned.ignore_files,
                    &inputs,
                    &from_profile,
                    &profile,
                )
                .map_err(|diagnostics| {
                    let described: Vec<String> = diagnostics.iter().map(describe).collect();
                    format!(
                        "--from-profile: the rules cannot read the profile's folder names: {}. Use other folder names",
                        described.join("; ")
                    )
                })?;
            if let Some(shell) = script {
                let root = absolute_folder(&directory)?;
                if let Some(undo) = &undo_script {
                    write_file(undo, &script::undo_script(&plan, shell, &root))?;
                }
                print!("{}", script::rename_script(&plan, shell, &root));
                return Ok(true);
            }
            match format {
                OutputFormat::Json => print_json(&to_json(&plan)?)?,
                OutputFormat::Table => print_plan(&plan),
            }
            Ok(true)
        }
        Command::Validate {
            directory,
            library,
            target_filesystem,
            all,
            fail_on,
            reference,
        } => {
            let reference: ReferenceMetadata = read_document(reference.as_deref())?;
            let scanned = scan_library(&directory, &rule_files)?;
            let interpretation =
                rules.interpret_with(library, &scanned.paths, &scanned.ignore_files, &inputs);
            let mut validation = rules.validate_with(
                &interpretation,
                &profile(base_profile, target_filesystem),
                &reference,
            );
            let fails = validation
                .diagnostics
                .iter()
                // Severities are ordered from most to least serious.
                .any(|diagnostic| diagnostic.severity <= fail_on);
            if !all {
                validation
                    .diagnostics
                    .retain(|diagnostic| diagnostic.severity != Severity::Info);
            }
            match format {
                OutputFormat::Json => print_json(&to_json(&validation)?)?,
                OutputFormat::Table => print_validation(&validation),
            }
            Ok(!fails)
        }
        Command::Diff {
            directory,
            library,
            rules: sides,
        } => {
            let [a, b] = sides.as_slice() else {
                return Err(format!(
                    "diff: `--rules` is given {} times. Pass exactly two rule sets",
                    sides.len()
                ));
            };
            let side = |side: &str| {
                let mut overlays = arguments.overlays.clone();
                if side != "builtin" {
                    overlays.push(PathBuf::from(side));
                }
                read_rules(&overlays, &arguments.languages)
            };
            let (a, b) = (side(a)?, side(b)?);
            // Reads the markers of both rule sets, so that each side sees its own markers.
            let rule_files = RuleFiles {
                names: rule_file_names,
                rules: vec![&a, &b],
            };
            let scanned = scan_library(&directory, &rule_files)?;
            let diff = nomenclator::diff_rules(
                &a,
                &b,
                library,
                &scanned.paths,
                &scanned.ignore_files,
                &inputs,
            );
            match format {
                OutputFormat::Json => print_json(&to_json(&diff)?)?,
                OutputFormat::Table => print_diff(&diff),
            }
            Ok(diff.paths.is_empty())
        }
    }
}

/// Fails if the reader does not read back a name that `profile` writes.
fn check_profile(rules: &Rules, profile: &OutputProfile) -> Result<(), String> {
    let problems: Vec<String> = rules
        .check_profile(profile)
        .iter()
        .map(|diagnostic| {
            format!(
                "output profile: {:?}: fact {} slot {:?}. Change the template of the slot",
                diagnostic.reason,
                diagnostic.fact.as_deref().unwrap_or("-"),
                diagnostic.slot,
            )
        })
        .collect();
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// Builds the built-in rules, or the rules of the declared languages, with the overlay
/// files applied. Fails with every overlay diagnostic if an overlay is invalid.
fn read_rules(paths: &[PathBuf], languages: &[String]) -> Result<Rules, String> {
    let mut overlays = Vec::new();
    let mut problems = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(path).map_err(|error| {
            format!("{}: cannot read the overlay file: {error}", path.display())
        })?;
        match Overlay::parse(&text) {
            Ok(overlay) => overlays.push(overlay),
            Err(diagnostics) => problems.extend(diagnostics.into_iter().map(|diagnostic| {
                let line = diagnostic
                    .line
                    .map(|line| format!(":{line}"))
                    .unwrap_or_default();
                format!("{}{line}: {}", path.display(), describe(&diagnostic))
            })),
        }
    }
    if problems.is_empty() {
        Rules::for_languages(languages, &overlays).map_err(|diagnostics| {
            diagnostics
                .iter()
                .map(describe)
                .collect::<Vec<_>>()
                .join("\n")
        })
    } else {
        Err(problems.join("\n"))
    }
}

fn describe(diagnostic: &nomenclator::OverlayDiagnostic) -> String {
    let mut parts = vec![vocabulary(diagnostic.reason)];
    parts.extend(
        diagnostic
            .overlay
            .iter()
            .map(|name| format!("overlay {name}")),
    );
    parts.extend(diagnostic.vocabulary.clone());
    parts.extend(diagnostic.entry.iter().map(|entry| format!("`{entry}`")));
    if !diagnostic.overlays.is_empty() {
        parts.push(format!("between {}", diagnostic.overlays.join(", ")));
    }
    parts.extend(diagnostic.message.clone());
    parts.join(": ")
}

/// Returns `base` with `target_filesystem` applied if it is set.
fn profile(base: OutputProfile, target_filesystem: Option<TargetFilesystem>) -> OutputProfile {
    let Some(target_filesystem) = target_filesystem else {
        return base;
    };
    OutputProfile {
        target_filesystem,
        ..base
    }
}

/// Reads a YAML or JSON document from `path`. Returns the default value if `path` is
/// `None`.
fn read_document<T: serde::de::DeserializeOwned + Default>(
    path: Option<&Path>,
) -> Result<T, String> {
    let Some(path) = path else {
        return Ok(T::default());
    };
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("{}: cannot read the file: {error}", path.display()))?;
    nomenclator::parse_document(&text).map_err(|error| {
        format!(
            "{}: cannot parse the YAML or JSON document: {error}",
            path.display()
        )
    })
}

/// Returns the rule-file names to read with their dialects: the defaults, then the
/// `NAME[:DIALECT]` arguments. Fails if an argument names an unknown dialect.
fn rule_file_names(extra: &[String]) -> Result<Vec<(String, IgnoreDialect)>, String> {
    let mut names = vec![(DEFAULT_RULE_FILE.to_owned(), IgnoreDialect::Gitignore)];
    for argument in extra {
        let (name, dialect) = match argument.rsplit_once(':') {
            Some((name, dialect)) => match IgnoreDialect::from_str(dialect, false) {
                Ok(dialect) => (name, dialect),
                Err(_) => {
                    let known: Vec<String> = IgnoreDialect::value_variants()
                        .iter()
                        .filter_map(|variant| {
                            Some(variant.to_possible_value()?.get_name().to_owned())
                        })
                        .collect();
                    return Err(format!(
                        "--rule-file {argument}: unknown dialect `{dialect}`. Use one of: {}",
                        known.join(", ")
                    ));
                }
            },
            None => (argument.as_str(), IgnoreDialect::Gitignore),
        };
        names.retain(|(known, _)| known != name);
        names.push((name.to_owned(), dialect));
    }
    Ok(names)
}

/// Lists every entry below `root` as a root-relative path, with folders ending in `/`,
/// and reads the rule files. Reports a name that is not UTF-8 and never decodes it
/// lossily (X4). Fails if `root` is not a folder.
fn scan_library(root: &Path, rule_files: &RuleFiles) -> Result<ScannedLibrary, String> {
    if !root.is_dir() {
        return Err(format!(
            "{}: the library path is not a folder. Pass the path of a library folder",
            root.display()
        ));
    }
    let mut scanned = ScannedLibrary::default();
    scan_folder(root, "", rule_files, &mut scanned);
    Ok(scanned)
}

fn scan_folder(
    folder: &Path,
    relative_folder: &str,
    rule_files: &RuleFiles,
    scanned: &mut ScannedLibrary,
) {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!(
                "{}: cannot read the folder ({error}). Its entries are skipped",
                folder.display()
            );
            return;
        }
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            eprintln!(
                "{}: unsupported_encoding: the name is not valid UTF-8. The entry is skipped",
                entry.path().display()
            );
            continue;
        };
        let file_type = entry.file_type();
        let is_symlink = file_type
            .as_ref()
            .is_ok_and(|file_type| file_type.is_symlink());
        // Follows links to tell a file from a folder, but does not descend into a linked
        // folder, so that a link cycle cannot cause endless recursion. The entry's own type
        // needs no further lookup unless the entry is a link.
        let is_folder = match file_type {
            Ok(file_type) if !file_type.is_symlink() => file_type.is_dir(),
            _ => entry.path().is_dir(),
        };
        if is_folder {
            let relative_path = format!("{relative_folder}{name}/");
            scanned.paths.push(relative_path.clone());
            if !is_symlink {
                scan_folder(&entry.path(), &relative_path, rule_files, scanned);
            }
            continue;
        }
        let relative_path = format!("{relative_folder}{name}");
        if let Some(dialect) = rule_files.dialect(name) {
            scanned.ignore_files.push(IgnoreFile {
                path: relative_path.clone(),
                contents: std::fs::read_to_string(entry.path()).ok(),
                dialect,
            });
        }
        scanned.paths.push(relative_path);
    }
}

fn print_interpretation(interpretation: &Interpretation) {
    let item_rows = interpretation.items.iter().map(item_row).collect();
    print_table(&["KIND", "IDENTITY", "LOCATION"], item_rows);
    let excluded = interpretation.excluded_entries.iter().map(|entry| {
        let rule_file = entry.rule_file.as_deref().unwrap_or("-");
        vec![
            entry.path.clone(),
            vocabulary(entry.reason),
            rule_file.to_owned(),
        ]
    });
    print_section(
        "Excluded",
        &["PATH", "REASON", "RULE FILE"],
        excluded.collect(),
    );
    let undecided = interpretation
        .undecided_entries
        .iter()
        .map(|entry| vec![entry.path.clone(), entry.unreadable_rule_files.join(", ")]);
    print_section(
        "Undecided",
        &["PATH", "UNREADABLE RULE FILES"],
        undecided.collect(),
    );
    let invalid = interpretation
        .invalid_entries
        .iter()
        .map(|entry| vec![entry.path.clone(), vocabulary(entry.reason)]);
    print_section("Invalid", &["PATH", "REASON"], invalid.collect());
    let unclaimed = interpretation
        .unclaimed_files
        .iter()
        .map(|path| vec![path.clone()]);
    print_section("Unclaimed", &["PATH"], unclaimed.collect());
    let diagnostics = interpretation.rule_diagnostics.iter().map(|diagnostic| {
        let line = diagnostic
            .line
            .map_or("-".to_owned(), |line| line.to_string());
        vec![
            diagnostic.rule_file.clone(),
            line,
            vocabulary(diagnostic.reason),
        ]
    });
    print_section(
        "Rule diagnostics",
        &["RULE FILE", "LINE", "REASON"],
        diagnostics.collect(),
    );
}

fn item_row(item: &Item) -> Vec<String> {
    let title_and_year = |title: &Option<String>, year: Option<u16>| {
        let title = title.as_deref().unwrap_or("?");
        year.map_or(title.to_owned(), |year| format!("{title} ({year})"))
    };
    let first_path = |paths: Option<&Vec<String>>| {
        paths
            .and_then(|paths| paths.first())
            .cloned()
            .unwrap_or_else(|| "-".to_owned())
    };
    let ambiguity = |ambiguous: bool, reasons: &[&str]| match ambiguous {
        true => format!(" [ambiguous: {}]", reasons.join(", ")),
        false => String::new(),
    };
    match item {
        Item::Movie(movie) => vec![
            "movie".to_owned(),
            title_and_year(&movie.title, movie.year)
                + &ambiguity(movie.ambiguous, &movie.ambiguity_reasons),
            movie.path.clone().unwrap_or_else(|| {
                first_path(movie.versions.first().map(|version| &version.paths))
            }),
        ],
        Item::Series(series) => vec![
            "series".to_owned(),
            title_and_year(&series.title, series.year),
            series.path.clone().unwrap_or_else(|| "-".to_owned()),
        ],
        Item::Season(season) => vec![
            "season".to_owned(),
            format!(
                "{} › season {}",
                season.series_name.as_deref().unwrap_or("?"),
                season.season
            ),
            season.path.clone().unwrap_or_else(|| "-".to_owned()),
        ],
        Item::Episode(episode) => vec![
            "episode".to_owned(),
            format!(
                "{} › {}{}",
                episode.series_name.as_deref().unwrap_or("?"),
                episode_coverage(episode),
                ambiguity(episode.ambiguous, &episode.ambiguity_reasons)
            ),
            first_path(episode.versions.first().map(|version| &version.paths)),
        ],
        Item::Extra(extra) => vec![
            "extra".to_owned(),
            format!(
                "{}: {}{}",
                vocabulary(extra.extra),
                title_and_year(&extra.title, extra.year),
                ambiguity(extra.ambiguous, &[])
            ),
            first_path(Some(&extra.paths)),
        ],
    }
}

/// Returns the episode's coverage: `S01E02E03`, `absolute 12`, or the air date.
fn episode_coverage(episode: &nomenclator::Episode) -> String {
    let season = episode
        .season
        .resolved_value()
        .map_or("S?".to_owned(), |season| format!("S{season:02}"));
    let episodes = episode.episodes.as_deref().unwrap_or_default();
    let absolute = episode.absolute.as_deref().unwrap_or_default();
    if !episodes.is_empty() {
        let codes: Vec<String> = episodes
            .iter()
            .map(|number| format!("E{number:02}"))
            .collect();
        format!("{season}{}", codes.concat())
    } else if !absolute.is_empty() {
        let numbers: Vec<String> = absolute.iter().map(u32::to_string).collect();
        format!("absolute {}", numbers.join(", "))
    } else if let Some(special) = &episode.special {
        let kind = vocabulary(special.kind);
        match &special.number {
            Some(number) => format!("S00 {kind} {number}"),
            None => format!("S00 {kind}"),
        }
    } else {
        episode
            .date
            .resolved_value()
            .cloned()
            .unwrap_or_else(|| "?".to_owned())
    }
}

fn print_import(plan: &ImportPlan) {
    let proposals = plan.proposals.iter().map(|proposal| {
        let mut notes = Vec::new();
        if let Some(video) = &proposal.moves_with {
            notes.push(format!("moves with {video}"));
        }
        if proposal.unclaimed {
            notes.push("unclaimed".to_owned());
        }
        vec![proposal.from.clone(), proposal.to.clone(), notes.join("; ")]
    });
    print_table(&["FROM", "TO", "NOTES"], proposals.collect());
    let blocked = plan.blocked.iter().map(|blocked| {
        let reasons: Vec<String> = blocked.reasons.iter().copied().map(vocabulary).collect();
        let to = blocked.to.clone().unwrap_or_else(|| "-".to_owned());
        vec![blocked.from.clone(), to, reasons.join(", ")]
    });
    print_section("blocked", &["FROM", "TO", "REASONS"], blocked.collect());
    let left = plan
        .left_in_place
        .iter()
        .map(|file| vec![file.path.clone(), vocabulary(file.reason)]);
    print_section("left in place", &["PATH", "REASON"], left.collect());
}

/// Returns `directory` as an absolute UTF-8 path. Fails if the path is not UTF-8.
fn absolute_folder(directory: &Path) -> Result<String, String> {
    let root = std::path::absolute(directory).map_err(|error| {
        format!(
            "{}: cannot resolve the folder: {error}",
            directory.display()
        )
    })?;
    root.to_str().map(str::to_owned).ok_or(format!(
        "{}: the folder path is not UTF-8. Pass a UTF-8 path",
        root.display()
    ))
}

fn write_file(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text)
        .map_err(|error| format!("{}: cannot write the file: {error}", path.display()))
}

fn print_plan(plan: &RenamePlan) {
    let proposals = plan.proposals.iter().map(|proposal| {
        let mut notes = Vec::new();
        if proposal.case_only {
            notes.push("case only".to_owned());
        }
        if proposal.requires_temporary_step {
            notes.push("needs a temporary name".to_owned());
        }
        if let Some(video) = &proposal.moves_with {
            notes.push(format!("moves with {video}"));
        }
        if proposal.unclaimed {
            notes.push("unclaimed".to_owned());
        }
        vec![proposal.from.clone(), proposal.to.clone(), notes.join("; ")]
    });
    print_table(&["FROM", "TO", "NOTES"], proposals.collect());
    let blocked = plan.blocked.iter().map(|blocked| {
        let reasons: Vec<String> = blocked.reasons.iter().copied().map(vocabulary).collect();
        let to = blocked.to.clone().unwrap_or_else(|| "-".to_owned());
        vec![blocked.from.clone(), to, reasons.join(", ")]
    });
    print_section(
        "Blocked",
        &["FROM", "WOULD BE", "REASONS"],
        blocked.collect(),
    );
    let removals = plan
        .removals
        .iter()
        .map(|removal| vec![removal.path.clone(), vocabulary(removal.reason)]);
    print_section("Proposed removals", &["PATH", "REASON"], removals.collect());
}

fn print_diff(diff: &nomenclator::RulesDiff) {
    let rows = diff.paths.iter().flat_map(|path| {
        path.facts.iter().map(move |fact| {
            let show = |value: &Option<String>| value.clone().unwrap_or_else(|| "-".to_owned());
            vec![
                path.path.clone(),
                fact.fact.clone(),
                show(&fact.a),
                show(&fact.b),
            ]
        })
    });
    print_table(&["PATH", "FACT", "A", "B"], rows.collect());
}

fn print_validation(validation: &Validation) {
    let rows = validation.diagnostics.iter().map(|diagnostic| {
        let mut detail: Vec<String> = diagnostic.reason.iter().map(vocabulary).collect();
        detail.extend(
            diagnostic
                .proposed_path
                .iter()
                .map(|path| format!("→ {path}")),
        );
        vec![
            vocabulary(diagnostic.severity),
            diagnostic.code.to_owned(),
            vocabulary(diagnostic.rule),
            diagnostic.paths.join(", "),
            detail.join(" "),
        ]
    });
    print_table(
        &["SEVERITY", "CODE", "RULE", "PATHS", "DETAIL"],
        rows.collect(),
    );
    let summary = &validation.summary;
    println!(
        "{} errors, {} warnings, {} infos, {} suppressed",
        summary.errors, summary.warnings, summary.infos, summary.suppressed
    );
}

/// Flattens facts into table rows. A finding row shows the value and evidence, or the
/// outcome if the finding is not resolved.
fn collect_fact_rows(field: &str, json: &Json, rows: &mut Vec<Vec<String>>) {
    let child = |name: &str| match field {
        "" => name.to_owned(),
        _ => format!("{field}.{name}"),
    };
    match json {
        Json::Object(fields) if fields.contains_key("status") => {
            let (value, evidence) = match fields["status"].as_str() {
                Some("resolved") => (
                    show_scalar(&fields["value"]),
                    fields.get("evidence").map_or(String::new(), |evidence| {
                        format!("{} {}", show_scalar(&evidence["rule"]), evidence["span"])
                    }),
                ),
                Some("ambiguous") => (
                    format!("ambiguous: {}", show_scalar(&fields["candidates"])),
                    show_scalar(&fields["reason"]),
                ),
                _ => ("unknown".to_owned(), String::new()),
            };
            rows.push(vec![field.to_owned(), value, evidence]);
        }
        Json::Object(fields) if !fields.is_empty() => {
            for (name, value) in fields {
                collect_fact_rows(&child(name), value, rows);
            }
        }
        Json::Array(items) if items.iter().any(Json::is_object) => {
            for (index, item) in items.iter().enumerate() {
                collect_fact_rows(&format!("{field}[{index}]"), item, rows);
            }
        }
        scalar => rows.push(vec![field.to_owned(), show_scalar(scalar), String::new()]),
    }
}

fn show_scalar(json: &Json) -> String {
    match json {
        Json::Null => "-".to_owned(),
        Json::String(text) => text.clone(),
        Json::Array(items) if items.is_empty() => "-".to_owned(),
        Json::Array(items) => items.iter().map(show_scalar).collect::<Vec<_>>().join(", "),
        Json::Object(fields) if fields.is_empty() => "-".to_owned(),
        other => other.to_string(),
    }
}

/// Returns the documented `snake_case` name of a vocabulary value.
fn vocabulary(value: impl Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|json| json.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn print_section(title: &str, headers: &[&str], rows: Vec<Vec<String>>) {
    if !rows.is_empty() {
        println!("\n{title}");
        print_table(headers, rows);
    }
}

fn print_table(headers: &[&str], rows: Vec<Vec<String>>) {
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| header.chars().count())
        .collect();
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let header_row: Vec<String> = headers.iter().map(|header| (*header).to_owned()).collect();
    for row in std::iter::once(&header_row).chain(&rows) {
        let cells: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        println!("{}", cells.join("  ").trim_end());
    }
}

fn to_json(result: impl Serialize) -> Result<Json, String> {
    serde_json::to_value(result).map_err(|error| error.to_string())
}

fn print_json(json: &Json) -> Result<(), String> {
    let text = serde_json::to_string_pretty(json).map_err(|error| error.to_string())?;
    println!("{text}");
    Ok(())
}
