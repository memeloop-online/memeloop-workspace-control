use super::*;

#[test]
fn request_labels_are_bounded_and_histograms_are_cumulative() {
    let metrics = Observability::default();
    let request = metrics.begin_http("BREW", "/api/v1/workspaces/{workspace_id}", 12);
    request.finish(503, 34);
    let snapshot = metrics.snapshot();
    let series = &snapshot.http[0];
    assert_eq!(series.method, "OTHER");
    assert_eq!(series.requests_by_status["5xx"], 1);
    assert_eq!(series.request_bytes_total, 12);
    assert_eq!(series.response_bytes_total, 34);
    assert!(
        series
            .duration_buckets
            .windows(2)
            .all(|pair| pair[0] <= pair[1])
    );
}

#[test]
fn stream_buffer_memory_is_released_with_the_stream() {
    let metrics = Observability::default();
    let mut stream = metrics.begin_sse();
    stream.set_buffer_bytes(512);
    assert_eq!(metrics.snapshot().active_sse, 1);
    assert_eq!(metrics.snapshot().sse_buffer_bytes, 512);
    stream.set_buffer_bytes(128);
    assert_eq!(metrics.snapshot().sse_buffer_bytes, 128);
    drop(stream);
    assert_eq!(metrics.snapshot().active_sse, 0);
    assert_eq!(metrics.snapshot().sse_buffer_bytes, 0);
}
