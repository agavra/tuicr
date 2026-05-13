use tuicr::review_api::{
    AddCommentRequest, FileDiffRequest, FileDiffView, GetReviewRequest, OpenReviewRequest,
    ReviewDiffSource, ReviewFileView, ReviewService, ReviewSessionView, SessionIdRequest,
    SetReviewedRequest,
};

#[test]
fn external_crates_can_import_review_api_surface() {
    let service = ReviewService::new(std::env::current_dir().expect("current dir"));
    let open_request = OpenReviewRequest {
        repo_path: None,
        diff_source: Some(ReviewDiffSource::WorkingTree),
        revisions: None,
        include_working_tree: None,
        path: None,
        file: None,
    };
    let diff_request = FileDiffRequest {
        session_id: "session".to_string(),
        path: "src/main.rs".into(),
        max_lines: Some(200),
    };
    let session_request = SessionIdRequest {
        session_id: "session".to_string(),
    };
    let _open_method: fn(
        &ReviewService,
        OpenReviewRequest,
    ) -> tuicr::error::Result<ReviewSessionView> = ReviewService::open_review;
    let _get_method: fn(
        &ReviewService,
        GetReviewRequest,
    ) -> tuicr::error::Result<ReviewSessionView> = ReviewService::get_review;
    let _diff_method: fn(&ReviewService, FileDiffRequest) -> tuicr::error::Result<FileDiffView> =
        ReviewService::get_file_diff;
    let _comment_method: fn(
        &ReviewService,
        AddCommentRequest,
    ) -> tuicr::error::Result<ReviewSessionView> = ReviewService::add_comment;
    let _reviewed_method: fn(
        &ReviewService,
        SetReviewedRequest,
    ) -> tuicr::error::Result<ReviewSessionView> = ReviewService::set_file_reviewed;
    let _clear_method: fn(
        &ReviewService,
        SessionIdRequest,
    ) -> tuicr::error::Result<ReviewSessionView> = ReviewService::clear_review;
    let _export_method: fn(&ReviewService, SessionIdRequest) -> tuicr::error::Result<String> =
        ReviewService::export_review;

    let file_view_size = std::mem::size_of::<ReviewFileView>();
    drop(service);
    drop(open_request);
    drop(diff_request);
    drop(session_request);
    assert!(file_view_size > 0);
}
