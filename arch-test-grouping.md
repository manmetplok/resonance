
**mixer** (34 files, 8908 LOC): audition_preview, automation_comp_delay, automation_live_value, automation_render, aux_send_render, clip_fade_gain_render, cycle_load, freeze_playback_substitution, graph_rate_assert, latency_comp, measure_mix, midi_event_cap, midi_event_window, midi_stash, mix_audio_parity, mixer_gain_ramp, monitor_fallback_resample, monitor_ring_alignment, recorded_monitor_gating, recorded_outbound_gating, recording_drain, recording_overflow_event, recording_start_latch, recording_whole_frame_push, reference_monitor, render_block_parity, sidechain_key_delivery, solo_predicate, stem_bus_sub_track_render, stem_render, stem_sub_track_render, sub_track_parent_fader, take_comp_render, underrun_rate_limiter

**engine** (23 files, 7240 LOC): automation_handlers, aux_send_cycle, bus_plugin_move, clip_fade_gain_handlers, clip_warp_handlers, deferred_clip_commands, device_params_handler, external_instrument_handlers, external_instrument_ping, external_recorded_playback, freeze_command_plumbing, loop_record_takes, master_plugin_move, midi_bulk_edits, midi_clip_handlers, midi_map_command_plumbing, offline_render_gate, playback_source_handler, playhead_seek_race, reference_handlers, take_removal, track_plugin_chain, track_plugin_move

**types** (14 files, 2983 LOC): aux_send_model, bar_length_shared, clip_warp, fade_curve, pw_latency_math, quantize_engine, sample_to_abs_tick, tempo_bar_at_sample_exact, tempo_map, tempo_position_to_bars, tempo_reanchor_math, transport_pos_beats, types_track, vocal_tuning_model

**bounce** (15 files, 3374 LOC): bounce_external_offsets, bounce_midi_events, bounce_plugin_lock, bounce_render_range_tempo, bounce_tail_and_master_latency, bounce_transport_guard, export_encoders, export_normalize, export_settings, freeze_cache_read, freeze_render_core, midi_export_project, reference_export_exclusion, stem_export, vocal_tuning_bounce

**clap_host** (15 files, 4094 LOC): clap_all_notes_off, clap_bundle_path, clap_factory_presets, clap_ffi_hardening, clap_latency_tracking, clap_note_event_order, clap_param_flush, clap_param_meta, clap_plugin_drop_order, plugin_bypass, plugin_editor_state, plugin_id_ranges, plugin_load_failure, plugin_output_scrub, plugin_rescan

**io** (8 files, 1718 LOC): clip_pitch_analysis, clip_tempo_detect, import_audio_to_pool, load_clip_offthread, load_wav_rate_mismatch, pw_output_smoke, reference_analysis, wav_chunk_parse

**midi_hw** (12 files, 2105 LOC): control_surface_parse, device_param_automation, live_arrival_offset, live_note_retry_order, midi_clock_parse, midi_hardware_emit, midi_hardware_parse, midi_io, midi_program_change, outbound_note_pairing, outbound_step_start, smf_import

**STANDALONE** (3 files, 590 LOC): engine_send_disconnected, recording_write_failure, sidechain_taps
