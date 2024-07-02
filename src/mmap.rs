use std::{
    io::ErrorKind,
    mem::transmute,
    os::raw::c_void,
};

const MMAP_PROT_FLAG: i32 = 3;
const MMAP_ANON_FLAG: i32 = 34;

extern "C" {
    fn mmap(
        addr: *const c_void,
        length: usize,
        prot: i32,
        flags: i32,
        fd: i32,
        offset: u64,
    ) -> *const c_void;
    fn munmap(add: *const c_void, length: usize) -> i32;
    fn getpagesize() -> usize;
}

pub fn mem_map(length: usize) -> Option<*const std::ffi::c_void> {
    unsafe {
        let block = mmap(
            0 as *const c_void,
            length,
            MMAP_PROT_FLAG,
            MMAP_ANON_FLAG,
            -1,
            0,
        );
        match block != transmute(-1 as isize)
        {
            true => Some(block),
            false => None,
        }
    }
}

pub fn mem_unmap(add: *const c_void, length: usize) -> Result<(), ErrorKind> {
    unsafe {
        match munmap(add, length) < 0 {
            true => Err(ErrorKind::Other),
            false => Ok(()),
        }
    }
}

pub fn get_page_size() -> usize {
    unsafe { getpagesize() }
}


#[cfg(test)]
mod map_tests{
    use std::ffi::c_void;

    use super::mem_unmap;

    #[test]
    #[should_panic]
    fn unmap_invalid() {
        let block = 56 as *const c_void;
        mem_unmap(block, 64).unwrap();
    }
}