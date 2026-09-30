//! Auto Layout helpers shared by the popover and the Settings window.
//!
//! Views size from their content: text measures itself, stacks add it up,
//! and the popover / window take the resulting fitting size. Nothing here
//! places a view at a coordinate.

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSLayoutAttribute, NSLayoutConstraint, NSStackView, NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{NSArray, NSEdgeInsets};

define_class!(
    /// A plain container with a top-left origin, so the scroll view's
    /// document starts at the first row instead of the bottom edge.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    pub(crate) struct FlippedView;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for FlippedView {}

    impl FlippedView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl FlippedView {
    pub(crate) fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: `NSView init` on a fresh allocation with no ivars.
        let this: Retained<Self> = unsafe { msg_send![Self::alloc(mtm), init] };
        manual(&this);
        this
    }
}

/// Opts a view out of autoresizing masks so its constraints are the whole
/// story.
pub(crate) fn manual(view: &NSView) {
    view.setTranslatesAutoresizingMaskIntoConstraints(false);
}

pub(crate) fn activate(constraints: &[Retained<NSLayoutConstraint>]) {
    let refs: Vec<&NSLayoutConstraint> = constraints.iter().map(|c| &**c).collect();
    NSLayoutConstraint::activateConstraints(&NSArray::from_slice(&refs));
}

/// Pins all four edges of `view` to `to`, inset by `inset` points.
pub(crate) fn pin_edges(
    view: &NSView,
    to: &NSView,
    inset: f64,
) -> Vec<Retained<NSLayoutConstraint>> {
    vec![
        view.topAnchor()
            .constraintEqualToAnchor_constant(&to.topAnchor(), inset),
        view.bottomAnchor()
            .constraintEqualToAnchor_constant(&to.bottomAnchor(), -inset),
        view.leadingAnchor()
            .constraintEqualToAnchor_constant(&to.leadingAnchor(), inset),
        view.trailingAnchor()
            .constraintEqualToAnchor_constant(&to.trailingAnchor(), -inset),
    ]
}

/// A top-to-bottom stack. Hidden arranged views drop out of the layout
/// (`detachesHiddenViews` is on by default), so the stack, and whatever
/// sizes itself from it, shrinks when a section has nothing to show.
pub(crate) fn column(mtm: MainThreadMarker, spacing: f64, inset: f64) -> Retained<NSStackView> {
    let stack = NSStackView::new(mtm);
    manual(&stack);
    stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    stack.setAlignment(NSLayoutAttribute::Leading);
    stack.setSpacing(spacing);
    stack.setEdgeInsets(NSEdgeInsets {
        top: inset,
        left: inset,
        bottom: inset,
        right: inset,
    });
    stack
}

/// Adds `view` to `stack` and makes it span the stack's width, minus the
/// stack's side insets.
pub(crate) fn add_full_width(stack: &NSStackView, view: &NSView, inset: f64) {
    manual(view);
    stack.addArrangedSubview(view);
    activate(&[view
        .widthAnchor()
        .constraintEqualToAnchor_constant(&stack.widthAnchor(), -2.0 * inset)]);
}
