//! The steps table of the Migration Tool (ADR 0046 "Crate `project_migrate` (D38)", techdoc §4.4): one entry per
//! project version after [`ProjectVersion::FIRST`], contiguous, the last one equal to [`ProjectVersion::CURRENT`].
//! Every later format change appends a step here and raises `CURRENT` by one.

use project_core::ProjectVersion;

/// One migration step: what the UI and `--dry-run` show for "upgrade the project to `to`".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepInfo {
    pub to: ProjectVersion,
    pub title: &'static str,
}

/// Every migration step, in order.
pub const STEPS: &[StepInfo] = &[
    StepInfo { to: ProjectVersion(2), title: "Rename format_version to project_version in project.ron" },
    // ADR 0047 §14: every domain migrates its own files (`MaterialMigrator`, `LightMigrator`).
    StepInfo { to: ProjectVersion(3), title: "Assign asset UUIDs and reference assets by UUID" },
];

/// Title of a step that is not in [`STEPS`] (synthetic targets past [`ProjectVersion::CURRENT`] in tests).
pub const GENERIC_STEP_TITLE: &str = "Version bump";

/// The title of the step to `to`.
pub fn step_title(to: ProjectVersion) -> &'static str {
    STEPS.iter().find(|s| s.to == to).map_or(GENERIC_STEP_TITLE, |s| s.title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_contiguous_up_to_current() {
        let mut expected = ProjectVersion::FIRST.next();
        for step in STEPS {
            assert_eq!(step.to, expected, "{step:?}");
            assert!(!step.title.is_empty() && !step.title.contains('\n'), "{step:?}");
            expected = expected.next();
        }
        assert_eq!(STEPS.last().map(|s| s.to), Some(ProjectVersion::CURRENT));
    }

    #[test]
    fn titles() {
        assert_eq!(step_title(ProjectVersion(2)), "Rename format_version to project_version in project.ron");
        assert_eq!(step_title(ProjectVersion(3)), "Assign asset UUIDs and reference assets by UUID");
        assert_eq!(step_title(ProjectVersion::CURRENT.next()), GENERIC_STEP_TITLE);
    }
}
