//! Traits that must be implemented by a wasm configuration blob.
//!
//! All the types are being exchanged bincode-encoded. If multiple
//! parameters are to be taken, they are exchanged as a
//! bincode-encoded tuple.
//!
//! `&mut` references taken as arguments are taken as though they were
//! by value, and then returned as supplementary arguments in a tuple.

pub mod server {
    pub use smtp_server_types::SerializableDecision;

    pub type ConnectionMetadata = smtp_server_types::ConnectionMetadata<Vec<u8>>;
    pub type MailMetadata = smtp_server_types::MailMetadata<Vec<u8>>;
}

// The functions are all implemented in wasm with:
//
// Parameters: (address, size) of the allocated block containing
// the serialized message. Ownership is passed to the called
// function.
//
// Return: u64 whose upper 32 bits are the size and lower 32 bits
// the address of a block containing the serialized message.
// Ownership is passed to the caller function.

// TODO: this all should be auto-generated by wasm-bindgen, wasm
// interface types, wiggle or similar

#[macro_export]
macro_rules! implement_host {
    () => {
        use std::{path::Path, rc::Rc};

        use anyhow::{anyhow, ensure, Context};

        // TODO: take struct name as argument instead of forcing the caller to put in a
        // mod (and same below)
        // TODO: factor code out with the below similar code to serialize the argument
        pub fn setup(
            path: &Path,
            instance: &wasmtime::Instance,
            allocate: Rc<dyn Fn(u32) -> Result<u32, wasmtime::Trap>>,
        ) -> anyhow::Result<()> {
            // Recover memory instance
            let memory = instance
                .get_memory("memory")
                .ok_or_else(|| anyhow!("Failed to find memory export ‘memory’"))?;

            // Recover setup function
            let wasm_fun = instance
                .get_func("setup")
                .ok_or_else(|| anyhow!("Failed to find function export ‘setup’"))?
                .get2()
                .with_context(|| format!("Checking the type of ‘setup’"))?;

            fn force_type<F: Fn(u32, u32) -> Result<(), wasmtime::Trap>>(_: &F) {}
            force_type(&wasm_fun);

            // Compute size of function
            let arg_size: u64 = bincode::serialized_size(path)
                .context("Figuring out size to allocate for argument buffer for ‘setup’")?;
            debug_assert!(
                arg_size <= u32::MAX as u64,
                "Message size above u32::MAX, something is really wrong"
            );
            let arg_size = arg_size as u32;

            // Allocate argument buffer
            let arg_ptr = allocate(arg_size).context("Allocating argument buffer for ‘setup’")?;
            ensure!(
                (arg_ptr as usize).saturating_add(arg_size as usize) < memory.data_size(),
                "Wasm allocator returned allocation outside of its memory"
            );

            // Serialize to argument buffer
            let arg_vec =
                bincode::serialize(path).context("Serializing argument buffer for ‘setup’")?;
            debug_assert_eq!(
                arg_size as usize,
                arg_vec.len(),
                "bincode-computed size is {} but actual size is {}",
                arg_size,
                arg_vec.len()
            );
            unsafe {
                std::intrinsics::volatile_copy_nonoverlapping_memory(
                    memory.data_ptr().add(arg_ptr as usize),
                    &arg_vec[0],
                    arg_size as usize,
                );
            }

            // Call the function
            let () = wasm_fun(arg_ptr, arg_size).context("Running wasm function ‘setup’")?;

            Ok(())
        }
    };
}

#[macro_export]
macro_rules! implement_guest {
    ($vis:vis trait $cfg_trait:ident, $cfg:ty) => {
        #[no_mangle]
        pub unsafe extern "C" fn allocate(size: usize) -> usize {
            // TODO: handle alloc error (ie. null return) properly (trap?)
            unsafe {
                std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(size, 8)) as usize
            }
        }

        #[no_mangle]
        pub unsafe extern "C" fn deallocate(ptr: usize, size: usize) {
            unsafe {
                std::alloc::dealloc(
                    ptr as *mut u8,
                    std::alloc::Layout::from_size_align_unchecked(size, 8),
                )
            }
        }

        $vis trait $cfg_trait {
            fn setup(path: std::path::PathBuf) -> Self;
        }

        std::thread_local! {
            static KANNADER_CFG: std::cell::RefCell<Option<$cfg>> =
                std::cell::RefCell::new(None);
        }

        // TODO: handle errors properly here too (see the TODO down the file)
        #[no_mangle]
        pub unsafe extern "C" fn setup(ptr: usize, size: usize) {
            // Recover the argument
            let arg_slice = std::slice::from_raw_parts(ptr as *const u8, size);
            let path: std::path::PathBuf = bincode::deserialize(arg_slice).unwrap();

            // Deallocate the memory block
            deallocate(ptr, size);

            // Run the code
            KANNADER_CFG.with(|cfg| {
                assert!(cfg.borrow().is_none());
                *cfg.borrow_mut() = Some(<$cfg as $cfg_trait>::setup(path));
            })
        }

        #[allow(unused)]
        fn DID_YOU_CALL_implement_guest_MACRO() {
            DID_YOU_CALL_server_config_implement_guest_MACRO();
        }
    };
}

macro_rules! define_communicator {
    (
        communicator
            $host_impler:ident
            $guest_impler:ident
            $did_you_call_fn_name:ident
        {
            $(
                $fn_name:ident =>
                    fn $fn:ident ( &self, $( $arg:ident : $mut:tt $ty:ty , )* ) -> $ret:ty ;
            )+
        }
    ) => {
        #[macro_export]
        macro_rules! $host_impler {
            (@mut_ref () $type:ty) => { $type };
            (@mut_ref (&mut) $type:ty) => { &mut $type };

            (@deref () $v:expr) => { $v };
            (@deref (&mut) $v:expr) => { *$v };

            (@if_mut () $e:expr) => { () };
            (@if_mut (&mut) $e:expr) => { $e };

            () => {
use $crate::$host_impler as implement_host;

use std::rc::Rc;

use anyhow::{anyhow, ensure, Context};

// TODO: take struct name as argument instead of forcing the caller to put in a mod (and same above)
pub struct HostSide {
    $(
        pub $fn: Box<dyn Fn($( implement_host!(@mut_ref $mut $ty) ),*) -> anyhow::Result<$ret>>,
    )+
}

#[allow(non_camel_case_types)]
pub fn build_host_side(
    instance: &wasmtime::Instance,
    allocate: Rc<dyn Fn(u32) -> Result<u32, wasmtime::Trap>>,
    deallocate: Rc<dyn Fn(u32, u32) -> Result<(), wasmtime::Trap>>
) -> anyhow::Result<HostSide> {
    let memory = instance
        .get_memory("memory")
        .ok_or_else(|| anyhow!("Failed to find memory export ‘memory’"))?;

    $(
        let $fn = {
            let memory = memory.clone();
            let allocate = allocate.clone();
            let deallocate = deallocate.clone();

            let wasm_fun = instance
                .get_func(stringify!($fn_name))
                .ok_or_else(|| anyhow!("Failed to find function export ‘{}’", stringify!($fn_name)))?
                .get2()
                .with_context(|| format!("Checking the type of ‘{}’", stringify!($fn_name)))?;

            fn force_type<F: Fn(u32, u32) -> Result<u64, wasmtime::Trap>>(_: &F) {}
            force_type(&wasm_fun);

            Box::new(move |$( $arg: implement_host!(@mut_ref $mut $ty) ),*| {
                // Get the to-be-encoded argument
                let arg = ( $( &implement_host!(@deref $mut $arg) ),* );

                // Compute the size of the argument
                let arg_size: u64 = bincode::serialized_size(&arg).with_context(|| {
                    format!(
                        "Figuring out size to allocate for argument buffer for ‘{}’",
                        stringify!($fn_name)
                    )
                })?;
                debug_assert!(
                    arg_size <= u32::MAX as u64,
                    "Message size above u32::MAX, something is really wrong"
                );
                let arg_size = arg_size as u32;

                // Allocate argument buffer
                let arg_ptr = allocate(arg_size)
                    .with_context(|| format!("Allocating argument buffer for ‘{}’", stringify!($fn_name)))?;
                ensure!(
                    (arg_ptr as usize).saturating_add(arg_size as usize) < memory.data_size(),
                    "Wasm allocator returned allocation outside of its memory"
                );

                // Serialize to argument buffer
                // TODO: implement io::Write for a VolatileWriter that directly
                // volatile-copies the message bytes to wasm memory
                let arg_vec = bincode::serialize(&arg)
                    .with_context(|| format!("Serializing argument buffer for ‘{}’", stringify!($fn_name)))?;
                debug_assert_eq!(
                    arg_size as usize,
                    arg_vec.len(),
                    "bincode-computed size is {} but actual size is {}",
                    arg_size,
                    arg_vec.len()
                );
                unsafe {
                    std::intrinsics::volatile_copy_nonoverlapping_memory(
                        memory.data_ptr().add(arg_ptr as usize),
                        &arg_vec[0],
                        arg_size as usize,
                    );
                }

                // Call the function
                let res_u64 = wasm_fun(arg_ptr, arg_size)
                    .with_context(|| format!("Running wasm function ‘{}’", stringify!($fn_name)))?;
                let res_ptr = (res_u64 & 0xFFFF_FFFF) as usize;
                let res_size = ((res_u64 >> 32) & 0xFFFF_FFFF) as usize;
                ensure!(
                    res_ptr.saturating_add(res_size) < memory.data_size(),
                    "Wasm function ‘{}’ returned allocation outside of its memory",
                    stringify!($fn_name),
                );

                // Recover the return slice
                // TODO: implement io::Read for a VolatileReader that directly volatile-copies
                // the message bytes from wasm memory
                let mut res_msg = vec![0; res_size];
                unsafe {
                    std::intrinsics::volatile_copy_nonoverlapping_memory(
                        &mut res_msg[0],
                        memory.data_ptr().add(res_ptr),
                        res_size,
                    );
                }

                // Deallocate the return slice
                deallocate(res_ptr as u32, res_size as u32)
                    .with_context(|| format!("Deallocating return buffer for function ‘{}’", stringify!($fn_name)))?;

                // Read the result
                let res;
                (res, $( implement_host!(@if_mut $mut *$arg) ),*) = bincode::deserialize(&res_msg)
                    .with_context(|| format!("Deserializing return message of ‘{}’", stringify!($fn_name)))?;
                 Ok(res)
            })
        };
    )+

    Ok(HostSide { $( $fn ),+ })
}
            };
        }


        #[macro_export]
        macro_rules! $guest_impler {
            (@mut_ref_ty () $type:ty) => { $type };
            (@mut_ref_ty (&mut) $type:ty) => { &mut $type };

            (@mut_ref_expr () $e:expr) => { $e };
            (@mut_ref_expr (&mut) $e:expr) => { &mut $e };

            (@mut_pat () $name:ident) => { $name };
            (@mut_pat (&mut) $name:ident) => { mut $name };

            (@if_mut () $e:expr) => { () };
            (@if_mut (&mut) $e:expr) => { $e };

            ($cfg:ty, $vis:vis trait $trait_name:ident, $impl_name:ty) => {
$vis trait $trait_name {
    $(
        fn $fn(cfg: & $cfg, $( $arg: $crate::$guest_impler!(@mut_ref_ty $mut $ty) ),*) -> $ret;
    )+
}

$(
    // TODO: handle errors properly (but what does “properly” exactly mean here?
    // anyway, probably not `.unwrap()` / `assert!`...) (and above in the file too)
    #[no_mangle]
    pub unsafe fn $fn_name(arg_ptr: usize, arg_size: usize) -> u64 {
        use $crate::$guest_impler as implement_guest;

        // Deserialize from the argument slice
        let arg_slice = std::slice::from_raw_parts(arg_ptr as *const u8, arg_size);
        let ( $( implement_guest!(@mut_pat $mut $arg) ),* ) =
            bincode::deserialize(arg_slice).unwrap();
         // Deallocate the argument slice
        deallocate(arg_ptr, arg_size);
         // Call the callback
        let res = KANNADER_CFG.with(|cfg| {
            <$impl_name as $trait_name>::$fn(
                cfg.borrow().as_ref().unwrap(),
                $( implement_guest!(@mut_ref_expr $mut $arg) ),*
            )
        });
        let res = (res, $( implement_guest!(@if_mut $mut $arg) ),*);

        // Allocate return buffer
        let ret_size: u64 = bincode::serialized_size(&res).unwrap();
        debug_assert!(
            ret_size <= usize::MAX as u64,
            "Message size above usize::MAX, something is really wrong"
        );
        let ret_size: usize = ret_size as usize;
        let ret_ptr: usize = allocate(ret_size);
        let ret_slice = std::slice::from_raw_parts_mut(ret_ptr as *mut u8, ret_size);

        // Serialize the result to the return buffer
        bincode::serialize_into(ret_slice, &res).unwrap();

        // We know that usize is u32 thanks to the above const_assert
        ((ret_size as u64) << 32) | (ret_ptr as u64)
    }
)+

#[allow(unused)]
fn $did_you_call_fn_name() {
    DID_YOU_CALL_implement_guest_MACRO();
}
            };
        }
    }
}

define_communicator! {
    communicator
        server_config_implement_host
        server_config_implement_guest
        DID_YOU_CALL_server_config_implement_guest_MACRO
    {
        server_config_filter_from => fn filter_from(
            &self,
            from: ( ) Option<smtp_message::Email>,
            meta: (&mut) smtp_server_types::MailMetadata<Vec<u8>>,
            conn_meta: (&mut) smtp_server_types::ConnectionMetadata<Vec<u8>>,
        ) -> smtp_server_types::SerializableDecision<Option<smtp_message::Email>>;
    }
}
