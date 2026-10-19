//! Resolve the Java service class on the Activity thread; reuse a global reference
//! from Python's async thread. No Android callback acquires the Python GIL.
#[cfg(target_os = "android")]
pub unsafe fn install(raw: *mut jni_sys::JNIEnv) {
    use accesskit_android::jni::{objects::{JClass, JString}, JavaVM};
    let Ok(mut env) = crate::a11y::bridge_env(raw) else { return };
    let result = (|| -> Result<_, accesskit_android::jni::errors::Error> {
        let class = env.find_class("dev/zkite/runtime/DeviceServices")?;
        let class = env.new_global_ref(class)?;
        let vm = env.get_java_vm()?.get_java_vm_pointer() as usize;
        Ok((class, vm))
    })();
    match result {
        Ok((class, vm)) => zkite_pybridge::set_device_exchange(Box::new(move |message| {
            let result = (|| -> Result<String, accesskit_android::jni::errors::Error> {
                let vm = unsafe { JavaVM::from_raw(vm as *mut _) }?;
                let mut env = vm.attach_current_thread()?;
                let result = env.with_local_frame(8, |env| {
                    let arg = env.new_string(message)?;
                    let cls: &JClass = class.as_obj().into();
                    let out = env.call_static_method(cls, "exchange", "(Ljava/lang/String;)Ljava/lang/String;", &[(&arg).into()])?.l()?;
                    let out = JString::from(out);
                    let text: String = env.get_string(&out)?.into();
                    Ok(text)
                });
                if result.is_err() { let _ = env.exception_clear(); }
                result
            })();
            result.unwrap_or_else(|e| serde_json::json!({"error":"DeviceUnavailable", "message":e.to_string()}).to_string())
        })),
        Err(e) => { let _ = env.exception_clear(); crate::loge!("device services: {e}"); }
    }
}
