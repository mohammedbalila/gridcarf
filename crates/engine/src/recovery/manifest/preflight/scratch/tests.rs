use super::*;

#[test]
fn geometric_requests_use_length_not_existing_capacity_and_check_before_reserve() {
    let mut values = Vec::<u32>::new();
    let mut requested = 0;
    let mut stats = Requests::default();
    reserve(&mut values, &mut requested, 4, 16, &mut stats).unwrap();
    values.push(1);
    reserve(&mut values, &mut requested, 8, 16, &mut stats).unwrap();
    assert!(values.capacity() >= 8);
    assert_eq!((requested, stats.calls, stats.current), (8, 2, 32));
    assert!(stats.peak >= 48 && stats.actual_peak >= 32);
    let calls = stats.calls;
    assert!(reserve(&mut values, &mut requested, 17, 16, &mut stats).is_err());
    assert_eq!(stats.calls, calls);
    let mut huge = Vec::<u64>::new();
    assert!(reserve(&mut huge, &mut 0, usize::MAX, usize::MAX, &mut stats).is_err());
    assert_eq!(stats.calls, calls);
    release(&values, requested, &mut stats);
    assert_eq!((stats.current, stats.actual_current), (0, 0));
}

#[test]
fn ancestor_keys_survive_child_release_and_detect_late_duplicates() {
    let mut keys = Keys::default();
    let mut requests = Requests::default();
    let limits = RecoveryLimits { manifest: 128, nodes: 32, ..RecoveryLimits::default() };
    let root = keys.mark();
    keys.push("parent", &limits, &mut requests).unwrap();
    let child = keys.mark();
    keys.push("same", &limits, &mut requests).unwrap();
    keys.check(child, &limits, &mut requests).unwrap();
    keys.release(child);
    keys.push("parent", &limits, &mut requests).unwrap();
    assert!(matches!(keys.check(root, &limits, &mut requests), Err(RecoveryError::Malformed("duplicate_key"))));
    keys.release(root);
    assert_eq!(keys.live(), (0, 0));
    assert!(keys.comparisons > 0);
    keys.close(&mut requests);
    assert_eq!((requests.current, requests.actual_current), (0, 0));
}

#[test]
fn exact_and_over_bound_key_requests_stop_before_copy() {
    let mut keys = Keys::default();
    let mut requests = Requests::default();
    let limits = RecoveryLimits { manifest: 4, nodes: 1, ..RecoveryLimits::default() };
    keys.push("four", &limits, &mut requests).unwrap();
    let calls = requests.calls;
    assert!(keys.push("x", &limits, &mut requests).is_err());
    assert_eq!((keys.live(), requests.calls), ((4, 1), calls));
}

#[test]
fn bottom_up_sort_handles_common_prefixes_and_duplicate_at_last_position() {
    let limits = RecoveryLimits::default();
    let mut keys = Keys::default();
    let mut requests = Requests::default();
    let scope = keys.mark();
    for index in (0..128).rev() {
        keys.push(&format!("{}{:03}", "x".repeat(100), index), &limits, &mut requests).unwrap();
    }
    keys.check(scope, &limits, &mut requests).unwrap();
    keys.push(&format!("{}000", "x".repeat(100)), &limits, &mut requests).unwrap();
    assert!(keys.check(scope, &limits, &mut requests).is_err());
}

#[test]
fn live_ancestor_inventory_merge_requests_and_work_are_observed() {
    let limits = RecoveryLimits::default();
    let mut keys = Keys::default();
    let mut requests = Requests::default();
    let root = keys.mark();
    keys.push("ancestor", &limits, &mut requests).unwrap();
    let child = keys.mark();
    for index in (0..64).rev() {
        keys.push(&format!("long-common-prefix-{index:03}"), &limits, &mut requests).unwrap();
    }
    keys.check(child, &limits, &mut requests).unwrap();
    assert_eq!(keys.live(), (8 + 64 * 22, 65));
    assert!(keys.comparisons > 0 && keys.comparisons <= 64 * 6);
    use std::io::Write;
    writeln!(
        std::io::stdout(),
        "C2_KEY_INVENTORY live={:?} requested={:?} capacity={:?} merge_len={} comparisons={} requests={:?}",
        keys.live(),
        keys.requested,
        [keys.bytes.capacity(), keys.spans.capacity(), keys.merge.capacity()],
        keys.merge.len(),
        keys.comparisons,
        requests
    )
    .unwrap();
    keys.release(child);
    assert_eq!(keys.live(), (8, 1));
    keys.push("later", &limits, &mut requests).unwrap();
    keys.check(root, &limits, &mut requests).unwrap();
    keys.release(root);
    keys.close(&mut requests);
    assert_eq!((requests.current, requests.actual_current), (0, 0));
}

#[test]
fn merge_limit_failure_is_a_real_request_path_before_resize_or_sort() {
    let mut values = vec![2u32, 1];
    let mut merge = Vec::new();
    let mut requested = 0;
    let mut stats = Requests::default();
    assert!(sort(&mut values, &mut merge, &mut requested, 1, &mut stats, Ord::cmp).is_err());
    assert_eq!((values.as_slice(), merge.len(), requested, stats.calls), (&[2, 1][..], 0, 0, 0));
    sort(&mut values, &mut merge, &mut requested, 2, &mut stats, Ord::cmp).unwrap();
    assert_eq!(values, [1, 2]);
    assert_eq!((merge.len(), requested, stats.calls), (2, 2, 1));
    release(&merge, requested, &mut stats);
    assert_eq!((stats.current, stats.actual_current), (0, 0));
}
