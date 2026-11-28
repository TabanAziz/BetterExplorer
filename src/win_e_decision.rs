// Kept independent of Windows calls so dispatch semantics can be checked separately.
pub(crate) fn dispatch(mask_events: u32, enqueue: impl FnOnce() -> bool) -> bool {
    mask_events == 2 && enqueue()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_input_does_not_queue_or_suppress_the_shortcut() {
        for sent in [0, 1] {
            assert!(!dispatch(sent, || panic!(
                "incomplete input must not queue"
            )));
        }
    }

    #[test]
    fn suppression_requires_an_accepted_request_after_both_mask_events() {
        assert!(!dispatch(2, || false));
        assert!(dispatch(2, || true));
    }
}
