//! The relationship access baseline: matrix rows from
//! `docs/test-plans/relationship_access_manual_testing.md`, sent through the production
//! route table on in-memory SQLite as each fixture caller.
//!
//! Rows assert only status and returned ids, never full bodies, and never depend on query
//! order, so moving a check between middleware and extractors cannot break them.
//!
//! Where each row is checked. `d/` is `domain/src/`; a bare `file::test` is in this module.
//!
//! Group A (REL, SES, SER, USR, ORG), group B (NOTE, AGR, ACT, GOAL) and group C (TOP, TRN,
//! MEET, SRCH, JWT, SSE, JOB, MAIL). `e/` is `entity_api/src/`, `w/` is `web/src/`:
//!
//! | Row | Where it is checked |
//! |---|---|
//! | REL-01 | `relationship_rows::rel_01_a_coach_lists_only_their_own_relationships` |
//! | REL-02 | `relationship_rows::rel_02_a_coachee_lists_only_their_own_relationships` |
//! | REL-03 | `relationship_rows::rel_03_another_coach_lists_only_their_relationship` |
//! | REL-04 | `relationship_rows::rel_04_a_member_in_no_relationship_lists_none` |
//! | REL-05 | `relationship_rows::rel_05_an_admin_lists_every_relationship_in_the_org` |
//! | REL-06 | `relationship_rows::rel_06_a_super_admin_lists_every_relationship_in_the_org` |
//! | REL-07 | `relationship_rows::rel_07_non_members_cannot_list_the_orgs_relationships` |
//! | REL-08 | `relationship_rows::rel_08_participants_read_their_relationship_with_names` |
//! | REL-09 | `relationship_rows::rel_09_non_participants_cannot_read_a_relationship` |
//! | REL-10 | `relationship_rows::rel_10_a_relationship_under_another_org_is_not_found` |
//! | REL-11 | `relationship_rows::rel_11_a_removed_coachee_cannot_read_their_relationship` |
//! | REL-12 | `relationship_rows::rel_12_a_coach_keeps_a_relationship_whose_coachee_left` |
//! | REL-13 | `relationship_rows::rel_13_a_coachee_lists_their_own_relationships_by_role` |
//! | REL-14 | `relationship_rows::rel_14_only_the_user_lists_their_own_relationships` |
//! | REL-15 | `relationship_rows::rel_15_admins_create_a_relationship_once_per_pair` |
//! | REL-16 | `relationship_rows::rel_16_non_admins_cannot_create_a_relationship` |
//! | REL-17 | `relationship_rows::rel_17_a_removed_coachee_lists_none_of_their_own_relationships`; `d/coaching_relationship_sqlite_tests.rs::a_users_own_relationships_skip_an_org_they_left` |
//! | REL-18 | `relationship_rows::rel_18_an_admin_sees_who_is_still_a_member` |
//! | SES-01 | `session_rows::ses_01_participants_list_the_relationships_sessions` |
//! | SES-02 | `session_rows::ses_02_non_participants_cannot_list_the_relationships_sessions` |
//! | SES-03 | `session_rows::ses_03_a_removed_coachee_cannot_list_their_relationships_sessions` |
//! | SES-04 | `session_rows::ses_04_participants_read_a_session_with_its_organization` |
//! | SES-05 | `session_rows::ses_05_non_participants_cannot_read_a_session` |
//! | SES-06 | `session_rows::ses_06_a_removed_coachee_cannot_read_their_session` |
//! | SES-07 | manual: the view upsert is Postgres-only SQL |
//! | SES-08 | `session_rows::ses_08_non_participants_cannot_mark_a_session_viewed` |
//! | SES-09 | `session_rows::ses_09_a_coach_creates_a_session_with_its_notes_document` |
//! | SES-10 | `session_rows::ses_10_a_coachee_cannot_create_a_session` |
//! | SES-11 | `session_rows::ses_11_members_outside_the_relationship_cannot_create_a_session` |
//! | SES-12 | `session_rows::ses_12_admins_cannot_create_a_session` |
//! | SES-13 | `session_rows::ses_13_an_outsider_cannot_create_a_session` |
//! | SES-14 | `session_rows::ses_14_a_removed_coachee_cannot_create_a_session` |
//! | SES-15 | `session_rows::ses_15_a_coach_updates_a_session` |
//! | SES-16 | `session_rows::ses_16_a_coachee_cannot_update_a_session` |
//! | SES-17 | `session_rows::ses_17_non_participants_cannot_update_a_session` |
//! | SES-18 | `session_rows::ses_18_a_coach_retitles_a_session_whose_coachee_left` |
//! | SES-19 | `session_rows::ses_19_either_participant_retitles_a_session` |
//! | SES-20 | `session_rows::ses_20_non_participants_cannot_retitle_a_session` |
//! | SES-21 | `session_rows::ses_21_a_coach_deletes_a_content_free_session` |
//! | SES-22 | `session_rows::ses_22_a_coachee_cannot_delete_a_session` |
//! | SES-23 | `session_rows::ses_23_non_participants_cannot_delete_a_session` |
//! | SES-24 | `session_rows::ses_24_a_coach_resends_a_sessions_invite` |
//! | SES-25 | `session_rows::ses_25_a_coachee_cannot_resend_a_sessions_invite` |
//! | SES-26 | `session_rows::ses_26_non_participants_cannot_resend_a_sessions_invite` |
//! | SES-27 | `session_rows::ses_27_a_coach_lists_their_own_sessions` |
//! | SES-28 | `session_rows::ses_28_a_coachee_lists_their_own_sessions` |
//! | SES-29 | `session_rows::ses_29_only_the_user_lists_their_own_sessions` |
//! | SES-30 | `session_rows::ses_30_a_removed_coachee_lists_none_of_their_own_sessions`; `d/coaching_session_sqlite_tests.rs::a_users_own_sessions_skip_an_org_they_left` |
//! | SES-31 | manual: the monthly counts are Postgres-only SQL |
//! | SES-32 | `session_rows::ses_32_a_coachee_reads_the_nearest_session_of_each_relationship`; `d/coaching_session_sqlite_tests.rs::relationships_the_caller_is_not_in_add_nothing`, `::each_relationship_keeps_its_own_window`, `::only_participants_read_the_window` |
//! | SES-33 | `session_rows::ses_33_scheduling_with_a_coachee_who_left_is_refused` |
//! | SES-34 | `session_rows::ses_34_moving_a_session_whose_coachee_left_is_refused` |
//! | SES-35 | `session_rows::ses_35_resending_to_a_coachee_who_left_is_refused` |
//! | SES-36 | `session_rows::ses_36_a_coach_deletes_a_session_whose_coachee_left` |
//! | SES-37 | `session_rows::ses_37_deleting_a_session_with_content_is_refused` |
//! | SER-01 | `series_rows::ser_01_a_coach_creates_a_series_with_its_sessions` |
//! | SER-02 | `series_rows::ser_02_a_coachee_cannot_create_a_series` |
//! | SER-03 | `series_rows::ser_03_non_participants_cannot_create_a_series` |
//! | SER-04 | `series_rows::ser_04_participants_list_the_relationships_series` |
//! | SER-05 | `series_rows::ser_05_non_participants_cannot_list_the_relationships_series` |
//! | SER-06 | `series_rows::ser_06_participants_read_a_series_with_its_sessions` |
//! | SER-07 | `series_rows::ser_07_non_participants_cannot_read_a_series` |
//! | SER-08 | manual: the series update takes Postgres advisory locks |
//! | SER-09 | `series_rows::ser_09_only_the_coach_reschedules_a_series` |
//! | SER-10 | manual: reschedule takes Postgres advisory locks |
//! | SER-11 | `series_rows::ser_11_only_the_coach_reschedules_following_sessions` |
//! | SER-12 | manual: the series delete takes Postgres advisory locks |
//! | SER-13 | `series_rows::ser_13_only_the_coach_deletes_a_series` |
//! | SER-14 | `series_rows::ser_14_a_coach_resends_a_series_invite` |
//! | SER-15 | `series_rows::ser_15_only_the_coach_resends_a_series_invite` |
//! | SER-16 | `series_rows::ser_16_a_coach_removed_from_the_org_cannot_create_a_series` |
//! | SER-17 | `series_rows::ser_17_a_series_with_a_coachee_who_left_is_refused` |
//! | SER-18 | `series_rows::ser_18_rescheduling_a_series_whose_coachee_left_is_refused` |
//! | SER-19 | manual: reschedule takes Postgres advisory locks before the membership check |
//! | SER-20 | `series_rows::ser_20_resending_a_series_to_a_coachee_who_left_is_refused` |
//! | SER-21 | manual: the series delete takes Postgres advisory locks |
//! | USR-01 | `user_rows::usr_01_a_coachees_sessions_include_their_relationships` |
//! | USR-02 | `user_rows::usr_02_a_coachee_coaches_no_one` |
//! | USR-03 | `user_rows::usr_03_a_coach_lists_the_relationships_they_coach` |
//! | ORG-01 | `organization_rows::org_01_admins_add_a_member_with_a_coach` |
//! | ORG-02 | `organization_rows::org_02_non_admins_cannot_add_a_member` |
//! | ORG-03 | `organization_rows::org_03_admins_attach_a_member_with_a_coach` |
//! | ORG-04 | `organization_rows::org_04_admins_remove_a_member_and_keep_their_relationship` |
//! | ORG-05 | `organization_rows::org_05_non_admins_cannot_remove_a_members_role` |
//! | ORG-06 | `organization_rows::org_06_admins_delete_a_member_without_coaching_history` |
//! | ORG-07 | `organization_rows::org_07_an_admin_cannot_delete_themselves` |
//! | ORG-08 | `organization_rows::org_08_deleting_a_member_with_coaching_history_is_refused` |
//! | NOTE-01 | `note_rows::note_01_participants_list_the_sessions_notes` |
//! | NOTE-02 | `note_rows::note_02_non_participants_cannot_list_the_sessions_notes` |
//! | NOTE-03 | `note_rows::note_03_a_removed_coachee_cannot_list_their_sessions_notes` |
//! | NOTE-04 | `note_rows::note_04_participants_read_the_note` |
//! | NOTE-05 | `note_rows::note_05_participants_create_a_note_in_the_session` |
//! | NOTE-06 | `note_rows::note_06_participants_update_the_note` |
//! | NOTE-07 | `note_rows::note_07_non_participants_cannot_read_the_note` |
//! | NOTE-08 | `note_rows::note_08_non_participants_cannot_create_a_note_in_the_session` |
//! | NOTE-09 | `note_rows::note_09_non_participants_cannot_update_the_note` |
//! | NOTE-10 | `note_rows::note_10_a_removed_coachee_cannot_reach_their_sessions_notes` |
//! | AGR-01 | `agreement_rows::agr_01_participants_list_the_sessions_agreements` |
//! | AGR-02 | `agreement_rows::agr_02_non_participants_cannot_list_the_sessions_agreements` |
//! | AGR-03 | `agreement_rows::agr_03_a_removed_coachee_cannot_list_their_sessions_agreements` |
//! | AGR-04 | `agreement_rows::agr_04_participants_read_the_agreement` |
//! | AGR-05 | `agreement_rows::agr_05_participants_create_an_agreement_in_the_session` |
//! | AGR-06 | `agreement_rows::agr_06_participants_update_the_agreement` |
//! | AGR-07 | `agreement_rows::agr_07_participants_delete_the_agreement` |
//! | AGR-08 | `agreement_rows::agr_08_non_participants_cannot_read_the_agreement` |
//! | AGR-09 | `agreement_rows::agr_09_non_participants_cannot_create_an_agreement_in_the_session` |
//! | AGR-10 | `agreement_rows::agr_10_non_participants_cannot_change_the_agreement` |
//! | AGR-11 | `agreement_rows::agr_11_a_removed_coachee_cannot_reach_their_sessions_agreements` |
//! | ACT-01 | `action_rows::act_01_a_coach_lists_actions_per_relationship` |
//! | ACT-02 | `action_rows::act_02_a_coachee_lists_only_own_and_unassigned_actions_per_relationship` |
//! | ACT-03 | `action_rows::act_03_another_coach_lists_only_their_relationships_actions` |
//! | ACT-04 | `action_rows::act_04_members_in_no_relationship_list_no_actions` |
//! | ACT-05 | `action_rows::act_05_non_members_cannot_list_the_orgs_actions` |
//! | ACT-06 | `action_rows::act_06_a_coachee_who_coaches_no_one_cannot_scope_to_the_coach` |
//! | ACT-07 | `action_rows::act_07_a_coach_lists_the_relationships_actions` |
//! | ACT-08 | `action_rows::act_08_a_coachee_lists_only_own_and_unassigned_relationship_actions` |
//! | ACT-09 | `action_rows::act_09_a_coachee_scopes_relationship_actions_only_to_themselves` |
//! | ACT-10 | `action_rows::act_10_non_participants_cannot_list_the_relationships_actions` |
//! | ACT-11 | `action_rows::act_11_an_outsider_cannot_list_the_relationships_actions` |
//! | ACT-12 | `action_rows::act_12_a_removed_coachee_cannot_list_their_relationships_actions` |
//! | ACT-13 | `action_rows::act_13_a_coach_lists_actions_of_a_relationship_whose_coachee_left` |
//! | ACT-14 | `action_rows::act_14_a_coach_lists_the_sessions_actions` |
//! | ACT-15 | `action_rows::act_15_a_coachee_lists_every_action_of_the_session` |
//! | ACT-16 | `action_rows::act_16_non_participants_cannot_list_the_sessions_actions` |
//! | ACT-17 | `action_rows::act_17_a_removed_coachee_cannot_list_their_sessions_actions` |
//! | ACT-18 | `action_rows::act_18_a_coachee_lists_every_action_of_their_sessions`; `d/action_sqlite_tests.rs::the_user_sees_every_relationship_they_are_in` |
//! | ACT-19 | `action_rows::act_19_a_coachee_lists_the_actions_assigned_to_them`; `d/action_sqlite_tests.rs::the_user_sees_every_assignment_of_theirs` |
//! | ACT-20 | `action_rows::act_20_a_coach_lists_their_own_actions` |
//! | ACT-21 | `action_rows::act_21_a_removed_coachee_lists_none_of_their_own_actions` |
//! | ACT-22 | `action_rows::act_22_a_coach_lists_only_their_relationships_actions_for_the_coachee`; `d/action_sqlite_tests.rs::a_coach_sees_only_their_relationships_with_the_user` |
//! | ACT-23 | `action_rows::act_23_a_coach_lists_only_their_relationships_assignments_of_the_coachee`; `d/action_sqlite_tests.rs::a_coach_sees_only_assignments_from_their_relationships` |
//! | ACT-24 | `action_rows::act_24_another_coach_lists_only_their_relationships_actions_for_the_coachee`; `d/action_sqlite_tests.rs::another_coach_sees_only_their_own_relationship` |
//! | ACT-25 | `action_rows::act_25_members_who_do_not_coach_the_user_cannot_list_their_actions` |
//! | ACT-26 | `action_rows::act_26_a_coachee_cannot_list_their_coachs_actions` |
//! | ACT-26b | `action_rows::act_26b_a_coach_removed_from_the_org_cannot_list_their_coachees_actions` |
//! | ACT-27 | `action_rows::act_27_participants_read_the_action` |
//! | ACT-28 | `action_rows::act_28_participants_create_an_action_assigned_to_the_coachee` |
//! | ACT-29 | `action_rows::act_29_participants_update_the_action` |
//! | ACT-30 | `action_rows::act_30_participants_update_the_action_status` |
//! | ACT-31 | `action_rows::act_31_participants_delete_the_action` |
//! | ACT-32 | `action_rows::act_32_non_participants_cannot_read_the_action` |
//! | ACT-33 | `action_rows::act_33_non_participants_cannot_create_an_action_in_the_session` |
//! | ACT-34 | `action_rows::act_34_non_participants_cannot_change_the_action` |
//! | ACT-35 | `action_rows::act_35_a_removed_coachee_cannot_reach_their_sessions_actions` |
//! | ACT-36 | `action_rows::act_36_assignees_outside_the_relationship_are_refused` |
//! | ACT-37 | `action_rows::act_37_assigning_a_coachee_who_left_is_refused` |
//! | ACT-38 | `action_rows::act_38_a_coach_edits_an_action_still_assigned_to_a_coachee_who_left` |
//! | ACT-39 | `action_rows::act_39_adding_a_coachee_who_left_as_an_assignee_is_refused` |
//! | ACT-40 | `action_rows::act_40_a_goal_from_another_relationship_is_refused` |
//! | GOAL-01 | `goal_rows::goal_01_participants_list_the_relationships_goals` |
//! | GOAL-02 | `goal_rows::goal_02_non_participants_cannot_list_the_relationships_goals` |
//! | GOAL-03 | `goal_rows::goal_03_a_removed_coachee_cannot_list_their_relationships_goals` |
//! | GOAL-04 | `goal_rows::goal_04_participants_read_the_goal` |
//! | GOAL-05 | `goal_rows::goal_05_non_participants_cannot_read_the_goal` |
//! | GOAL-06 | `goal_rows::goal_06_participants_update_the_goal` |
//! | GOAL-07 | `goal_rows::goal_07_participants_delete_the_goal` |
//! | GOAL-08 | `goal_rows::goal_08_non_participants_cannot_change_the_goal` |
//! | GOAL-09 | `goal_rows::goal_09_participants_list_the_goals_sessions` |
//! | GOAL-10 | `goal_rows::goal_10_participants_create_a_goal_in_the_relationship` |
//! | GOAL-11 | `goal_rows::goal_11_participants_list_the_sessions_goals` |
//! | GOAL-12 | `goal_rows::goal_12_non_participants_cannot_list_the_sessions_goals` |
//! | GOAL-13 | `goal_rows::goal_13_participants_link_a_goal_to_another_session_of_the_relationship` |
//! | GOAL-14 | `goal_rows::goal_14_participants_unlink_a_goal_from_the_session` |
//! | GOAL-15 | `goal_rows::goal_15_participants_read_goals_per_session_of_the_relationship` |
//! | GOAL-16 | `goal_rows::goal_16_non_participants_cannot_read_goals_per_session_of_the_relationship` |
//! | GOAL-17 | `goal_rows::goal_17_every_batch_session_must_be_the_callers` |
//! | GOAL-18 | `goal_rows::goal_18_non_participants_cannot_batch_read_the_sessions_goals` |
//! | GOAL-19 | `goal_rows::goal_19_participants_read_the_relationships_goal_progress` |
//! | GOAL-20 | `goal_rows::goal_20_goal_progress_refuses_a_non_participant_assignee` |
//! | GOAL-21 | `goal_rows::goal_21_non_participants_cannot_read_the_relationships_goal_progress` |
//! | GOAL-22 | `goal_rows::goal_22_a_coach_lists_the_goals_they_created` |
//! | GOAL-23 | `goal_rows::goal_23_a_coachee_who_created_no_goal_lists_none` |
//! | GOAL-24 | `goal_rows::goal_24_only_the_user_lists_their_own_goals` |
//! | GOAL-25 | `goal_rows::goal_25_non_participants_cannot_create_a_goal_in_the_relationship` |
//! | GOAL-26 | `goal_rows::goal_26_non_participants_cannot_change_the_sessions_goal_links` |
//! | GOAL-27 | `goal_rows::goal_27_linking_a_goal_from_another_relationship_is_refused` |
//! | GOAL-28 | `goal_rows::goal_28_a_created_in_session_from_another_relationship_is_refused` |
//! | TOP-01 | `topic_rows::top_01_participants_list_the_sessions_topics` |
//! | TOP-02 | `topic_rows::top_02_non_participants_cannot_list_the_sessions_topics` |
//! | TOP-03 | `topic_rows::top_03_participants_add_a_topic_authored_by_themselves` |
//! | TOP-04 | `topic_rows::top_04_non_participants_cannot_add_a_topic` |
//! | TOP-05 | `topic_rows::top_05_participants_reorder_the_sessions_topics` |
//! | TOP-06 | `topic_rows::top_06_a_coach_edits_the_coachees_topic` |
//! | TOP-07 | `topic_rows::top_07_a_coachee_edits_the_coachs_topic` |
//! | TOP-08 | `topic_rows::top_08_non_participants_cannot_edit_a_topic` |
//! | TOP-09 | `topic_rows::top_09_a_coach_deletes_the_coachees_topic` |
//! | TOP-10 | `topic_rows::top_10_a_coachee_deletes_their_own_topic` |
//! | TOP-11 | `topic_rows::top_11_a_coachee_cannot_delete_the_coachs_topic` |
//! | TOP-12 | `topic_rows::top_12_non_participants_cannot_delete_a_topic` |
//! | TOP-13 | removed by rs#489 |
//! | TOP-14 | removed by rs#489 |
//! | TOP-15 | removed by rs#489 |
//! | TOP-16 | `topic_rows::top_16_participants_set_a_topics_status` |
//! | TOP-17 | `topic_rows::top_17_non_participants_cannot_set_a_topics_status` |
//! | TOP-18 | `topic_rows::top_18_only_the_author_undoes_a_topics_delete` |
//! | TOP-19 | `topic_rows::top_19_the_author_undoes_a_topics_delete` |
//! | TOP-20 | `topic_rows::top_20_participants_undo_a_topics_defer` |
//! | TOP-21 | `topic_rows::top_21_non_participants_cannot_undo_a_topic_change` |
//! | TRN-01 | `transcript_rows::trn_01_participants_read_the_latest_transcription` |
//! | TRN-02 | `transcript_rows::trn_02_non_participants_cannot_read_the_latest_transcription` |
//! | TRN-03 | `transcript_rows::trn_03_participants_download_only_the_coachs_lines` |
//! | TRN-04 | `transcript_rows::trn_04_non_participants_cannot_read_a_transcription` |
//! | TRN-05 | `transcript_rows::trn_05_participants_read_the_transcriptions_segments` |
//! | TRN-06 | `transcript_rows::trn_06_non_participants_cannot_read_the_transcriptions_segments` |
//! | TRN-07 | `transcript_rows::trn_07_participants_list_the_sessions_suggestions` |
//! | TRN-08 | `transcript_rows::trn_08_non_participants_cannot_list_the_sessions_suggestions` |
//! | TRN-09 | `transcript_rows::trn_09_participants_read_and_start_a_scan` (the POST runs on a fresh unscanned R1 session; S1's finished scan answers 409) |
//! | TRN-10 | `transcript_rows::trn_10_non_participants_cannot_read_or_start_a_scan` |
//! | TRN-11 | `transcript_rows::trn_11_a_coach_accepts_a_coach_only_suggestion` |
//! | TRN-12 | `transcript_rows::trn_12_a_coachee_cannot_accept_a_coach_only_suggestion` |
//! | TRN-13 | `transcript_rows::trn_13_a_coachee_accepts_their_own_suggestion` |
//! | TRN-14 | `transcript_rows::trn_14_non_participants_cannot_accept_a_suggestion` |
//! | TRN-15 | `transcript_rows::trn_15_a_coach_dismisses_and_restores_a_coach_only_suggestion` |
//! | TRN-16 | `transcript_rows::trn_16_a_coachee_cannot_dismiss_or_restore_a_coach_only_suggestion` |
//! | TRN-17 | `transcript_rows::trn_17_non_participants_cannot_dismiss_or_restore_a_suggestion` |
//! | TRN-18 | `transcript_rows::trn_18_participants_read_the_latest_recording` |
//! | TRN-19 | manual: Recall base URL is hard-coded |
//! | TRN-20 | manual: stopping the bot needs Recall, whose base URL is hard-coded |
//! | TRN-21 | `transcript_rows::trn_21_non_participants_cannot_reach_the_sessions_recording` |
//! | TRN-22 | `transcript_rows::trn_22_a_removed_coachee_cannot_reach_their_sessions_recording` |
//! | TRN-23 | `transcript_rows::trn_23_participants_upload_a_note_image` |
//! | TRN-24 | `transcript_rows::trn_24_non_participants_cannot_upload_a_note_image` |
//! | TRN-25 | `transcript_rows::trn_25_participants_read_a_note_image` |
//! | TRN-26 | `transcript_rows::trn_26_either_participant_deletes_and_restores_a_note_image` |
//! | TRN-27 | `transcript_rows::trn_27_non_participants_cannot_read_or_delete_a_note_image` |
//! | TRN-28 | `transcript_rows::trn_28_accepting_for_a_coachee_who_left_is_refused` |
//! | MEET-01 | manual: first-read hydration takes `pg_advisory_xact_lock` and creates the link with the provider |
//! | MEET-02 | `meeting_rows::meet_02_only_the_coachs_create_attaches_a_meeting_link` (a reused R1 link; a new provider link needs Google or Zoom) |
//! | MEET-03 | `meeting_rows::meet_03_a_coach_sets_the_sessions_meeting_link` |
//! | MEET-04 | manual: Recall base URL is hard-coded |
//! | MEET-05 | `meeting_rows::meet_05_every_caller_reaches_only_their_own_connections` |
//! | SRCH-01 | manual: full-text search |
//! | SRCH-02 | manual: full-text search |
//! | SRCH-03 | manual: full-text search |
//! | SRCH-04 | `search_rows::srch_04_members_outside_every_relationship_find_nothing` |
//! | SRCH-05 | manual: full-text search |
//! | SRCH-06 | `search_rows::srch_06_a_removed_coachee_finds_nothing` |
//! | SRCH-07 | manual: full-text search |
//! | SRCH-08 | `search_rows::srch_08_a_relationship_filter_never_widens_a_members_search` |
//! | SRCH-09 | manual: full-text search |
//! | SRCH-10 | manual: full-text search |
//! | JWT-01 | `jwt_rows::jwt_01_participants_get_a_token_for_the_sessions_document` |
//! | JWT-02 | `jwt_rows::jwt_02_non_participants_get_no_token` |
//! | JWT-03 | `jwt_rows::jwt_03_a_removed_coachee_gets_no_token` |
//! | SSE-01 | `sse_rows::sse_01_every_caller_opens_an_event_stream` |
//! | SSE-02 | `sse_rows::sse_02_both_participants_hear_of_an_action_assigned_to_the_coach` |
//! | SSE-03 | `sse_rows::sse_03_a_removed_coachee_hears_nothing_of_their_relationship` |
//! | SSE-04 | `sse_rows::sse_04_both_participants_hear_of_a_goal_change` |
//! | SSE-05 | `sse_rows::sse_05_both_participants_hear_that_suggestions_changed` (through a scan start, which shares the sweep's `extract::notify`); `d/transcript_suggestion/extract_sqlite_tests.rs::every_completed_run_announces_once_with_or_without_suggestions` |
//! | SSE-06 | `sse_rows::sse_06_both_participants_hear_of_a_recording_update` |
//! | JOB-01 | manual: raw Postgres SQL; SQL shape in `e/coaching_session.rs::tests::claim_due_reminders_skips_former_organization_members`, `::claim_due_reminders_membership_mirrors_is_member`, `::claim_due_reminders_skips_sessions_booked_on_short_notice` |
//! | JOB-02 | `d/jobs/coaching_session_pending_reschedule_sqlite_tests.rs::a_due_change_is_emailed_and_settled`, `::a_coachee_no_longer_in_the_organization_is_not_emailed` |
//! | JOB-03 | `d/transcript_suggestion/prompt_tests.rs::render_writes_the_session_existing_actions_and_transcript`; events as SSE-05 |
//! | JOB-04 | `d/jobs/coaching_session_image_purge_sqlite_tests.rs` (no recipients) |
//! | JOB-05 | `d/password_reset.rs::tests::sweep_old_attempts_returns_deleted_count` (no recipients) |
//! | MAIL-01 | `d/emails_tests.rs::relationship_emails_reach_only_current_members`; `d/emails_sqlite_tests.rs::session_scheduled_with_the_coachee_holding_no_role_reaches_only_the_coach` |
//! | MAIL-02 | `d/emails_tests.rs::test_send_session_rescheduled_email`; `d/jobs/coaching_session_pending_reschedule_sqlite_tests.rs::a_coachee_no_longer_in_the_organization_is_not_emailed` |
//! | MAIL-03 | `d/emails_sqlite_tests.rs::single_cancel_with_the_coachee_holding_no_role_reaches_both`, `::single_cancel_with_the_coach_holding_no_role_reaches_both`, `::a_series_edit_cancels_an_orphan_for_a_coachee_holding_no_role` |
//! | MAIL-04 | `mail_rows::mail_04_a_session_resend_mails_both_members`; `w/controller/resend_invite_tests.rs::session_resend_bumps_a_one_off`, `::resend_with_a_removed_coach_mails_only_the_coachee`, `::resend_to_a_removed_coachee_is_refused` |
//! | MAIL-05 | `d/emails_tests.rs::relationship_emails_reach_only_current_members`; `d/coaching_session_series.rs::tests::m5_08_series_create_invites_only_current_members` |
//! | MAIL-06 | `d/emails_tests.rs::relationship_emails_reach_only_current_members`; `d/coaching_session_series.rs::tests::m5_09_series_reschedule_emails_only_current_members` |
//! | MAIL-07 | `d/emails_tests.rs::relationship_emails_reach_only_current_members`; `d/coaching_session_series.rs::tests::m5_10_reschedule_following_emails_only_current_members` |
//! | MAIL-08 | `d/emails_sqlite_tests.rs::series_cancel_with_the_coachee_holding_no_role_reaches_both` |
//! | MAIL-09 | `mail_rows::mail_09_a_series_resend_mails_both_members`; `w/controller/resend_invite_tests.rs::series_resend_bumps_the_series_and_raises_its_members`, `::resend_with_a_removed_coach_mails_only_the_coachee`, `::resend_to_a_removed_coachee_is_refused` |
//! | MAIL-10 | `d/emails_tests.rs::test_send_session_reminder_email_goes_only_to_the_coachee` |
//! | MAIL-11 | `d/emails_tests.rs::action_assigned_reaches_only_current_members` |
//! | MAIL-12 | `d/emails_tests.rs::test_send_welcome_email_success` |
//! | MAIL-13 | `d/emails_added_to_organization_tests.rs::send_added_to_organization_email_wire_contract` |
//! | MAIL-14 | `d/emails_tests.rs::test_send_password_reset_email_wire_contract` |

mod harness;

mod action_rows;
mod agreement_rows;
mod goal_rows;
mod jwt_rows;
mod mail_rows;
mod meeting_rows;
mod note_rows;
mod organization_rows;
mod relationship_rows;
mod search_rows;
mod series_rows;
mod session_rows;
mod sse_rows;
mod topic_rows;
mod transcript_rows;
mod user_rows;
