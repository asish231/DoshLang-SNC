use crate::ir::IrModule;

pub fn emit_llvm(module: &IrModule, triple: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("target triple = \"{triple}\"\n\n"));
    s.push_str(DECLS);
    s.push('\n');
    for (i, lit) in module.strings.iter().enumerate() {
        let (esc, n) = llvm_escape(lit);
        s.push_str(&format!(
            "@.str.{i} = private unnamed_addr constant [{n} x i8] c\"{esc}\"\n"
        ));
    }
    s.push('\n');
    for g in &module.globals {
        s.push_str(g);
        s.push('\n');
    }
    for f in &module.functions {
        s.push_str(f);
        s.push('\n');
    }
    if module.has_main {
        s.push_str("define i32 @main(i32 %argc, ptr %argv) {\n");
        s.push_str("  call void @sn_rt_init(i32 %argc, ptr %argv)\n");
        if module.main_ret_void {
            s.push_str("  call void @sn_fn_main()\n");
        } else {
            s.push_str("  call i64 @sn_fn_main()\n");
        }
        s.push_str("  call void @sn_rt_shutdown()\n");
        s.push_str("  ret i32 0\n");
        s.push_str("}\n");
    }
    s
}

fn llvm_escape(s: &str) -> (String, usize) {
    let mut out = String::new();
    let mut n = 0usize;
    for b in s.as_bytes() {
        match *b {
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\22"),
            0..=31 | 127..=255 => out.push_str(&format!("\\{b:02X}")),
            c => out.push(c as char),
        }
        n += 1;
    }
    out.push_str("\\00");
    n += 1;
    (out, n)
}

const DECLS: &str = r#"
declare ptr @sn_str_new(ptr, i64)
declare ptr @sn_str_from_cstr(ptr)
declare i64 @sn_str_len(ptr)
declare ptr @sn_str_cstr(ptr)
declare ptr @sn_str_concat(ptr, ptr)
declare i64 @sn_str_eq(ptr, ptr)
declare ptr @sn_str_slice(ptr, i64, i64)
declare i64 @sn_str_contains(ptr, ptr)
declare ptr @sn_str_split(ptr, ptr)
declare ptr @sn_str_replace(ptr, ptr, ptr)
declare ptr @sn_str_upper(ptr)
declare ptr @sn_str_lower(ptr)
declare ptr @sn_str_from_f64(double)
declare double @sn_f64_from_str(ptr)
declare void @sn_print_f64(double)
declare void @sn_printn_f64(double)
declare ptr @sn_str_from_i64(i64)
declare ptr @sn_str_from_bool(i64)
declare ptr @sn_str_from_dec(i64, i64)
declare i64 @sn_i64_from_str(ptr)
declare void @sn_print_i64(i64)
declare void @sn_printn_i64(i64)
declare void @sn_print_bool(i64)
declare void @sn_printn_bool(i64)
declare void @sn_print_str(ptr)
declare void @sn_printn_str(ptr)
declare void @sn_print_none()
declare void @sn_printn_none()
declare ptr @sn_list_new(i64)
declare void @sn_list_push_i64(ptr, i64)
declare void @sn_list_push_ptr(ptr, ptr)
declare i64 @sn_list_len(ptr)
declare ptr @sn_list_data(ptr)
declare i64 @sn_list_get_i64(ptr, i64)
declare ptr @sn_list_get_ptr(ptr, i64)
declare void @sn_list_set_i64(ptr, i64, i64)
declare void @sn_list_set_ptr(ptr, i64, ptr)
declare ptr @sn_map_new(i64, i64)
declare void @sn_map_set(ptr, i64, i64)
declare i64 @sn_map_get(ptr, i64)
declare i64 @sn_map_has(ptr, i64)
declare i64 @sn_map_len(ptr)
declare ptr @sn_map_keys(ptr)
declare ptr @sn_map_values(ptr)
declare ptr @sn_input(ptr)
declare ptr @sn_file_read(ptr)
declare i64 @sn_file_write(ptr, ptr)
declare void @sn_file_read_ex(ptr, ptr, ptr)
declare ptr @sn_file_write_ex(ptr, ptr)
declare ptr @sn_file_append(ptr, ptr)
declare i64 @sn_file_exists(ptr)
declare ptr @sn_file_delete(ptr)
declare ptr @sn_file_copy(ptr, ptr)
declare ptr @sn_file_move(ptr, ptr)
declare ptr @sn_file_mkdir(ptr)
declare ptr @sn_file_rmdir(ptr)
declare void @sn_file_list(ptr, ptr, ptr)
declare ptr @sn_path_join(ptr, ptr)
declare ptr @sn_path_base(ptr)
declare ptr @sn_path_dir(ptr)
declare ptr @sn_path_ext(ptr)
declare ptr @sn_os_getenv(ptr)
declare void @sn_os_exit(i64)
declare ptr @sn_os_args()
declare i64 @sn_list_contains_i64(ptr, i64)
declare i64 @sn_list_contains_ptr(ptr, ptr)
declare ptr @sn_error_new(ptr)
declare ptr @sn_error_msg(ptr)
declare void @sn_panic(ptr)
declare ptr @sn_alloc(i64)
declare void @sn_free(ptr)
declare ptr @sn_box_i64(i64)
declare i64 @sn_unbox_i64(ptr)
declare i64 @sn_pow_i64(i64, i64)
declare i64 @sn_div_i64(i64, i64)
declare i64 @sn_mod_i64(i64, i64)
declare ptr @sn_obj_new(i64, i64, ptr)
declare ptr @sn_chan_new()
declare void @sn_chan_send(ptr, i64)
declare i64 @sn_chan_recv(ptr)
declare void @sn_chan_close(ptr)
declare i64 @sn_chan_open(ptr)
declare i64 @sn_chan_select(i64, ptr, i64)
declare ptr @sn_lock_new()
declare void @sn_lock_acq(ptr)
declare void @sn_lock_rel(ptr)
declare void @sn_spawn(ptr, ptr)
declare void @sn_go_spawn(ptr, ptr)
declare void @sn_rt_init(i32, ptr)
declare void @sn_rt_shutdown()
declare void @sn_retain(ptr)
declare void @sn_release(ptr)
declare ptr @sn_clos_new(ptr, ptr)
declare ptr @sn_clos_fn(ptr)
declare ptr @sn_clos_env(ptr)
declare void @sn_json_parse(ptr, ptr, ptr)
declare ptr @sn_json_encode(ptr)
declare ptr @sn_json_as_str(ptr)
declare i64 @sn_json_as_int(ptr)
declare i64 @sn_json_as_bool(ptr)
declare i64 @sn_json_is_null(ptr)
declare i64 @sn_json_is_object(ptr)
declare i64 @sn_json_is_list(ptr)
declare i64 @sn_json_len(ptr)
declare ptr @sn_json_get(ptr, ptr)
declare ptr @sn_json_at(ptr, i64)
declare ptr @sn_json_keys(ptr)
declare void @sn_http_request(ptr, ptr, ptr, ptr, ptr)
declare ptr @sn_http_serve_once(i64, ptr)
declare i64 @sn_obj_type_id(ptr)
declare void @sn_record_copy(ptr, ptr, i64)
declare ptr @sn_any_box_i64(i64)
declare i64 @sn_any_as_int(ptr)
declare ptr @sn_any_as_str(ptr)
declare i64 @sn_any_as_bool(ptr)
declare ptr @sn_any_type_name(ptr)
declare ptr @sn_async_sleep(i64)
declare ptr @sn_async_http_get(ptr)
declare i64 @sn_hton16(i64)
declare i64 @sn_ntoh16(i64)
declare i64 @sn_hton32(i64)
declare i64 @sn_ntoh32(i64)
declare i64 @sn_hton64(i64)
declare i64 @sn_ntoh64(i64)
declare i64 @sn_swap16_i64(i64)
declare i64 @sn_swap32_i64(i64)
declare i64 @sn_swap64_i64(i64)
declare i64 @sn_future_await(ptr)
declare ptr @sn_future_data(ptr)
declare i64 @sn_tcp_listen(i64)
declare i64 @sn_tcp_listen_any(i64)
declare i64 @sn_tcp_local_port(i64)
declare i64 @sn_tcp_connect(ptr, i64, i64)
declare i64 @sn_tcp_accept(i64)
declare i64 @sn_tcp_read(i64)
declare i64 @sn_tcp_write(i64, ptr, i64)
declare void @sn_tcp_close(i64)
declare i64 @sn_select_read(i64, ptr, i64)
declare i64 @sn_ptr_load(ptr, i64)
declare void @sn_ptr_store(ptr, i64, i64)
declare i64 @sn_ptr_load_byte(ptr, i64)
declare void @sn_ptr_store_byte(ptr, i64, i64)
declare double @sn_ptr_load_f64(ptr, i64)
declare void @sn_ptr_store_f64(ptr, i64, double)
declare ptr @sn_ptr_load_str(ptr, i64)
declare void @sn_ptr_store_str(ptr, i64, ptr, i64)
declare ptr @sn_ptr_alloc(i64)
declare ptr @sn_bytearray_new(i64)
declare i64 @sn_bytearray_len(ptr)
declare void @sn_bytearray_reserve(ptr, i64)
declare i64 @sn_bytearray_push(ptr, i64)
declare i64 @sn_bytearray_get(ptr, i64)
declare void @sn_bytearray_set(ptr, i64, i64)
declare void @sn_bytearray_append_bytes(ptr, ptr, i64)
declare void @sn_bytearray_append_str(ptr, ptr)
declare ptr @sn_bytearray_slice(ptr, i64, i64)
declare i64 @sn_bytearray_write_at(ptr, i64, ptr, i64)
declare i64 @sn_bytearray_find(ptr, i64, i64)
declare i64 @sn_bytearray_rfind(ptr, i64)
declare void @sn_bytearray_truncate(ptr, i64)
declare void @sn_bytearray_set_len(ptr, i64)
declare void @sn_bytearray_clear(ptr)
declare ptr @sn_bytearray_data(ptr)
declare void @sn_bytearray_fill(ptr, i64, i64)
declare void @sn_bytearray_sha256(ptr, i64, ptr)
declare void @sn_bytearray_md5(ptr, i64, ptr)
declare void @sn_bytearray_sha512(ptr, i64, ptr)
declare void @sn_bytearray_hmac_sha256(ptr, i64, ptr, i64, ptr)
declare i64 @sn_crc32(ptr, i64)
declare i64 @sn_crc32_update(i64, ptr, i64)
declare i64 @sn_consttime_eq(ptr, ptr, i64)
declare void @sn_secure_zero(ptr, i64)
declare i64 @sn_gc_object_count()
declare void @sn_gc_collect()
declare void @sn_gc_root_push(ptr)
declare void @sn_gc_collect_roots()
declare void @sn_gc_set_threshold(i64)
declare void @sn_sha256(ptr, i64, ptr)
declare void @sn_sha512(ptr, i64, ptr)
declare void @sn_md5(ptr, i64, ptr)
declare void @sn_hmac_sha256(ptr, i64, ptr, i64, ptr)
declare void @sn_ptr_free(ptr)
declare i64 @sn_free_checked(ptr)
declare i64 @sn_check_live(ptr)
declare i64 @sn_alloc_size(ptr)
declare i64 @sn_alloc_live_count()
declare i64 @sn_alloc_freed_count()
declare void @sn_alloc_guard(i64)
declare i64 @sn_time_now_ms()
declare void @sn_time_sleep_ms(i64)
declare ptr @sn_time_format(i64)
"#;
