//! Writes shell scripts that perform a rename or import plan, and scripts that reverse
//! it (6.3, 6.4).
//!
//! A script moves the proposals in plan order. A proposal in a rename cycle, or one that
//! changes only letter case, first moves to a temporary name, so that no move overwrites
//! a file and a case-insensitive filesystem renames the file. A script stops at the first
//! destination that exists. Proposed removals are written as comments, and the script
//! removes nothing: the folders that a script empties stay in place.

use clap::ValueEnum;
use nomenclator::{ImportPlan, RenamePlan};

/// The shell that a rename script is written for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    /// A POSIX shell script.
    Sh,
    /// A PowerShell script.
    Ps1,
}

/// One move of a script: a path relative to the source root to a path relative to the
/// target root.
struct Move<'a> {
    from: &'a str,
    to: &'a str,
    temporary: bool,
}

/// Returns the script that performs `plan` in the library at `root`, an absolute path.
pub fn rename_script(plan: &RenamePlan, shell: Shell, root: &str) -> String {
    let moves: Vec<Move> = plan
        .proposals
        .iter()
        .map(|proposal| Move {
            from: &proposal.from,
            to: &proposal.to,
            temporary: proposal.requires_temporary_step || proposal.case_only,
        })
        .collect();
    let removals: Vec<String> = plan
        .removals
        .iter()
        .map(|removal| format!("proposed removal: {}", removal.path))
        .collect();
    write_script(&moves, &removals, shell, root, root)
}

/// Returns the script that reverses `plan` in the library at `root`, an absolute path.
/// It moves each destination back to its source, in reverse plan order.
pub fn undo_script(plan: &RenamePlan, shell: Shell, root: &str) -> String {
    let moves: Vec<Move> = plan
        .proposals
        .iter()
        .rev()
        .map(|proposal| Move {
            from: &proposal.to,
            to: &proposal.from,
            temporary: proposal.requires_temporary_step || proposal.case_only,
        })
        .collect();
    write_script(&moves, &[], shell, root, root)
}

/// Returns the script that performs `plan`: it moves files from the download at
/// `download` into the library at `library`. Both are absolute paths.
pub fn import_script(plan: &ImportPlan, shell: Shell, download: &str, library: &str) -> String {
    let moves: Vec<Move> = plan
        .proposals
        .iter()
        .map(|proposal| Move {
            from: &proposal.from,
            to: &proposal.to,
            temporary: false,
        })
        .collect();
    write_script(&moves, &[], shell, download, library)
}

/// Returns the script that reverses `plan`: it moves each imported file back from the
/// library at `library` into the download at `download`.
pub fn import_undo_script(
    plan: &ImportPlan,
    shell: Shell,
    download: &str,
    library: &str,
) -> String {
    let moves: Vec<Move> = plan
        .proposals
        .iter()
        .rev()
        .map(|proposal| Move {
            from: &proposal.to,
            to: &proposal.from,
            temporary: false,
        })
        .collect();
    write_script(&moves, &[], shell, library, download)
}

/// Returns the script that performs `moves` from `source_root` to `target_root`.
/// Temporary names are in the source root.
fn write_script(
    moves: &[Move],
    comments: &[String],
    shell: Shell,
    source_root: &str,
    target_root: &str,
) -> String {
    let temporary_names: Vec<Option<String>> = moves
        .iter()
        .enumerate()
        .map(|(index, step)| {
            step.temporary
                .then(|| format!("{}.nomenclator-{}.tmp", step.from, index + 1))
        })
        .collect();
    let mut script = match shell {
        Shell::Sh => sh_header(source_root, target_root),
        Shell::Ps1 => ps1_header(source_root, target_root),
    };
    for comment in comments {
        script.push_str(&format!("# {}\n", comment.replace('\n', " ")));
    }
    // Every temporary move runs first, so that each source of a cycle is free before
    // any file moves into it.
    let temporary_moves = moves
        .iter()
        .zip(&temporary_names)
        .filter_map(|(step, temporary)| Some((step.from, temporary.as_deref()?, false)));
    let final_moves = moves
        .iter()
        .zip(&temporary_names)
        .map(|(step, temporary)| (temporary.as_deref().unwrap_or(step.from), step.to, true));
    for (from, to, into_target) in temporary_moves.chain(final_moves) {
        let line = match shell {
            Shell::Sh => {
                let root = if into_target { "TARGET" } else { "SOURCE" };
                format!(
                    "move \"$SOURCE\"/{} \"${root}\"/{}\n",
                    sh_quote(from),
                    sh_quote(to)
                )
            }
            Shell::Ps1 => {
                let root = if into_target { "$Target" } else { "$Source" };
                format!(
                    "Move-Entry (Join-Path $Source {}) (Join-Path {root} {})\n",
                    ps1_quote(from),
                    ps1_quote(to)
                )
            }
        };
        script.push_str(&line);
    }
    script
}

fn sh_header(source_root: &str, target_root: &str) -> String {
    format!(
        r#"#!/bin/sh
# Written by nomenclator. Each path is relative to SOURCE or TARGET.
set -eu
SOURCE={source}
TARGET={target}
move() {{
  if [ -e "$2" ] || [ -L "$2" ]; then
    echo "nomenclator: $2 exists. The script stops before it moves $1." >&2
    exit 1
  fi
  mkdir -p -- "$(dirname -- "$2")"
  mv -- "$1" "$2"
}}
"#,
        source = sh_quote(source_root),
        target = sh_quote(target_root)
    )
}

fn ps1_header(source_root: &str, target_root: &str) -> String {
    format!(
        r#"# Written by nomenclator. Each path is relative to $Source or $Target.
$ErrorActionPreference = 'Stop'
$Source = {source}
$Target = {target}
function Move-Entry([string]$From, [string]$To) {{
  if (Test-Path -LiteralPath $To) {{
    throw "nomenclator: $To exists. The script stops before it moves $From."
  }}
  [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($To)) | Out-Null
  if (Test-Path -LiteralPath $From -PathType Container) {{
    [System.IO.Directory]::Move($From, $To)
  }} else {{
    [System.IO.File]::Move($From, $To)
  }}
}}
"#,
        source = ps1_quote(source_root),
        target = ps1_quote(target_root)
    )
}

/// Quotes `text` for a POSIX shell: single quotes, with each `'` written as `'\''`.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Quotes `text` for PowerShell: single quotes, with each `'` doubled. PowerShell also
/// reads the typographic single quotes as quotes, so they are doubled too.
fn ps1_quote(text: &str) -> String {
    let mut quoted = String::from("'");
    for character in text.chars() {
        if matches!(
            character,
            '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}'
        ) {
            quoted.push(character);
        }
        quoted.push(character);
    }
    quoted.push('\'');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomenclator::{RenameProposal, Rules};

    fn plan(proposals: &[(&str, &str, bool, bool)]) -> RenamePlan {
        RenamePlan {
            proposals: proposals
                .iter()
                .map(|(from, to, cycle, case_only)| RenameProposal {
                    from: (*from).to_owned(),
                    to: (*to).to_owned(),
                    case_only: *case_only,
                    requires_temporary_step: *cycle,
                    moves_with: None,
                    unclaimed: false,
                })
                .collect(),
            blocked: Vec::new(),
            removals: Vec::new(),
            rules: Rules::builtin().identity().clone(),
        }
    }

    fn moves(script: &str) -> Vec<&str> {
        script
            .lines()
            .filter(|line| line.starts_with("move ") || line.starts_with("Move-Entry "))
            .collect()
    }

    #[test]
    fn rename_script_moves_in_plan_order() {
        let plan = plan(&[("a.mkv", "A (2016)/A (2016).mkv", false, false)]);
        let script = rename_script(&plan, Shell::Sh, "/media/movies");
        assert!(script.contains("SOURCE='/media/movies'"));
        assert_eq!(
            moves(&script),
            [r#"move "$SOURCE"/'a.mkv' "$TARGET"/'A (2016)/A (2016).mkv'"#]
        );
    }

    #[test]
    fn rename_script_moves_cycle_through_temporary_names() {
        let plan = plan(&[
            ("a.mkv", "b.mkv", true, false),
            ("b.mkv", "a.mkv", true, false),
        ]);
        let script = rename_script(&plan, Shell::Sh, "/media");
        assert_eq!(
            moves(&script),
            [
                r#"move "$SOURCE"/'a.mkv' "$SOURCE"/'a.mkv.nomenclator-1.tmp'"#,
                r#"move "$SOURCE"/'b.mkv' "$SOURCE"/'b.mkv.nomenclator-2.tmp'"#,
                r#"move "$SOURCE"/'a.mkv.nomenclator-1.tmp' "$TARGET"/'b.mkv'"#,
                r#"move "$SOURCE"/'b.mkv.nomenclator-2.tmp' "$TARGET"/'a.mkv'"#,
            ]
        );
    }

    #[test]
    fn rename_script_moves_case_only_change_through_temporary_name() {
        let plan = plan(&[("arrival.mkv", "Arrival.mkv", false, true)]);
        let script = rename_script(&plan, Shell::Ps1, "C:/Media");
        assert_eq!(
            moves(&script),
            [
                "Move-Entry (Join-Path $Source 'arrival.mkv') (Join-Path $Source 'arrival.mkv.nomenclator-1.tmp')",
                "Move-Entry (Join-Path $Source 'arrival.mkv.nomenclator-1.tmp') (Join-Path $Target 'Arrival.mkv')",
            ]
        );
    }

    #[test]
    fn undo_script_moves_destinations_back_in_reverse_order() {
        let plan = plan(&[
            ("b/x.mkv", "c/x.mkv", false, false),
            ("a/x.mkv", "b/x.mkv", false, false),
        ]);
        let script = undo_script(&plan, Shell::Sh, "/media");
        assert_eq!(
            moves(&script),
            [
                r#"move "$SOURCE"/'b/x.mkv' "$TARGET"/'a/x.mkv'"#,
                r#"move "$SOURCE"/'c/x.mkv' "$TARGET"/'b/x.mkv'"#,
            ]
        );
    }

    #[test]
    fn sh_quote_escapes_single_quote() {
        assert_eq!(sh_quote("Schindler's List"), r"'Schindler'\''s List'");
    }

    #[test]
    fn ps1_quote_doubles_single_quotes() {
        assert_eq!(ps1_quote("Schindler's List"), "'Schindler''s List'");
        assert_eq!(ps1_quote("It\u{2019}s"), "'It\u{2019}\u{2019}s'");
    }
}
