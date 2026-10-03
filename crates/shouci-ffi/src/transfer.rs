//! Import and export previews. A plan stays in Rust as an object, so Swift
//! applies exactly what it previewed: an export's rendered file never
//! crosses over, and an import plan cannot be edited between preview and
//! apply.

use std::sync::Arc;

use shouci_core::{
    ExportPlan, ImportCounts, ImportPlan, ImportPolicy, Issue, PlannedLine, Severity,
};

/// A line-level finding from reading a file.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct IssueView {
    /// 1-based.
    pub line: u32,
    pub severity: Severity,
    pub message: String,
}

impl From<&Issue> for IssueView {
    fn from(issue: &Issue) -> Self {
        Self {
            line: issue.line,
            severity: issue.severity,
            message: issue.message.clone(),
        }
    }
}

/// An import plan, as shown before applying it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ImportPlanView {
    pub connector_id: String,
    pub format: String,
    pub path: String,
    pub policy: ImportPolicy,
    /// The file has error lines; applying imports nothing.
    pub refused: bool,
    pub issues: Vec<IssueView>,
    pub lines: Vec<PlannedLine>,
    pub notes: Vec<String>,
    pub counts: ImportCounts,
}

/// A file read as the format that fits it best.
#[derive(uniffi::Record)]
pub struct DetectedImport {
    pub preview: Arc<ImportPreview>,
    /// No other format reads the file as well.
    pub unambiguous: bool,
}

#[derive(uniffi::Object)]
pub struct ImportPreview {
    plan: ImportPlan,
}

impl ImportPreview {
    pub(crate) fn new(plan: ImportPlan) -> Self {
        Self { plan }
    }

    pub(crate) fn plan(&self) -> &ImportPlan {
        &self.plan
    }
}

#[uniffi::export]
impl ImportPreview {
    #[must_use]
    pub fn view(&self) -> ImportPlanView {
        let plan = &self.plan;
        ImportPlanView {
            connector_id: plan.connector_id.clone(),
            format: plan.format.clone(),
            path: plan.path.clone(),
            policy: plan.policy,
            refused: plan.refused,
            issues: plan.issues.iter().map(IssueView::from).collect(),
            lines: plan.lines.clone(),
            notes: plan.notes.clone(),
            counts: plan.counts(),
        }
    }
}

/// An export plan, as shown before writing it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ExportPlanView {
    pub connector_id: String,
    pub format: String,
    pub path: String,
    pub item_ids: Vec<i64>,
    /// Headwords, oldest first.
    pub words: Vec<String>,
    pub left_out_needs_review: u32,
    pub left_out_already_there: u32,
    /// A file is already at `path` and will be replaced.
    pub replaces_existing: bool,
    /// What the format could not carry.
    pub notes: Vec<String>,
}

#[derive(uniffi::Object)]
pub struct ExportPreview {
    plan: ExportPlan,
}

impl ExportPreview {
    pub(crate) fn new(plan: ExportPlan) -> Self {
        Self { plan }
    }

    pub(crate) fn plan(&self) -> &ExportPlan {
        &self.plan
    }
}

#[uniffi::export]
impl ExportPreview {
    #[must_use]
    pub fn view(&self) -> ExportPlanView {
        let plan = &self.plan;
        ExportPlanView {
            connector_id: plan.connector_id.clone(),
            format: plan.format.clone(),
            path: plan.path.clone(),
            item_ids: plan.item_ids.clone(),
            words: plan.words.clone(),
            left_out_needs_review: plan.left_out_needs_review,
            left_out_already_there: plan.left_out_already_there,
            replaces_existing: plan.replaces_existing,
            notes: plan.notes.clone(),
        }
    }
}
