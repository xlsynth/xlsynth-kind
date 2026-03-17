use std::ffi::{CStr, CString};
use std::ptr;

use xlsynth::XlsynthError;

pub fn emit_ir_function_as_smtlib(
    ir_text: &str,
    function_name: &str,
) -> Result<String, XlsynthError> {
    let ir_text = CString::new(ir_text)
        .map_err(|err| XlsynthError(format!("IR text contained interior NUL: {err}")))?;
    let filename = CString::new("roundtrip.ir").unwrap();
    let function_name = CString::new(function_name)
        .map_err(|err| XlsynthError(format!("function name contained interior NUL: {err}")))?;

    unsafe {
        let mut error_out = ptr::null_mut();
        let mut package_out = ptr::null_mut();
        if !xlsynth_sys::xls_parse_ir_package(
            ir_text.as_ptr(),
            filename.as_ptr(),
            &mut error_out,
            &mut package_out,
        ) {
            return Err(XlsynthError(take_c_string(error_out)));
        }
        let package = PackageGuard(package_out);

        let mut function_out = ptr::null_mut();
        if !xlsynth_sys::xls_package_get_function(
            package.0,
            function_name.as_ptr(),
            &mut error_out,
            &mut function_out,
        ) {
            return Err(XlsynthError(take_c_string(error_out)));
        }

        let mut smtlib_out = ptr::null_mut();
        if !xlsynth_sys::xls_function_to_z3_smtlib(function_out, &mut error_out, &mut smtlib_out) {
            return Err(XlsynthError(take_c_string(error_out)));
        }

        Ok(take_c_string(smtlib_out))
    }
}

struct PackageGuard(*mut xlsynth_sys::CIrPackage);

impl Drop for PackageGuard {
    fn drop(&mut self) {
        unsafe {
            xlsynth_sys::xls_package_free(self.0);
        }
    }
}

unsafe fn take_c_string(ptr: *mut std::os::raw::c_char) -> String {
    let text = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe {
        xlsynth_sys::xls_c_str_free(ptr);
    }
    text
}
