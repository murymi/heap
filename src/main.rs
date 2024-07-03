use std::{
    ffi::c_void,
    mem::{self, discriminant},
    ptr::{self, null},
};

use mmap::{mem_map, mem_unmap};

mod mmap;
const PAGE_SIZE: usize = 4096;
const TINY_HEAP_ALLOCATION_SIZE: usize = 4 * PAGE_SIZE;
const TINY_BLOCK_SIZE: usize = TINY_HEAP_ALLOCATION_SIZE / 128;
const SMALL_HEAP_ALLOCATION_SIZE: usize = 32 * PAGE_SIZE;
const SMALL_BLOCK_SIZE: usize = SMALL_HEAP_ALLOCATION_SIZE / 128;

#[derive(Debug, PartialEq)]
#[repr(u8)]
#[repr(C)]
enum HeapGroup {
    Tiny(usize),
    Small(usize),
    Large(usize),
}

impl From<usize> for HeapGroup {
    fn from(value: usize) -> Self {
        if value <= TINY_BLOCK_SIZE {
            Self::Tiny(value)
        } else if value <= SMALL_BLOCK_SIZE {
            Self::Small(value)
        } else {
            Self::Large(value)
        }
    }
}

impl HeapGroup {
    fn alloc_size(&self) -> usize {
        match self {
            HeapGroup::Tiny(_) => TINY_HEAP_ALLOCATION_SIZE,
            HeapGroup::Small(_) => SMALL_HEAP_ALLOCATION_SIZE,
            HeapGroup::Large(v) => v + mem::size_of::<Block>() + mem::size_of::<Heap>(),
        }
    }
}

#[derive(Debug)]
#[repr(C)]
struct Heap {
    group: HeapGroup,
    next: *mut Heap,
    previous: *mut Heap,
    total_size: usize,
    free_size: usize,
    block_count: usize,
}

impl Heap {
    fn new(size: usize) -> Self {
        let gp: HeapGroup = size.into();
        let size = gp.alloc_size();
        Self {
            next: 0 as *mut Heap,
            previous: 0 as *mut Heap,
            total_size: size,
            free_size: size - Self::size(),
            group: gp,
            block_count: 0,
        }
    }

    fn size() -> usize {
        mem::size_of::<Self>()
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct Block {
    next: *const Block,
    previous: *const Block,
    data_size: usize,
    free: bool,
}

impl Block {
    fn new(size: usize) -> Self {
        Self {
            next: 0 as *const Block,
            previous: 0 as *const Block,
            data_size: size,
            free: false,
        }
    }

    fn size() -> usize {
        return mem::size_of::<Block>();
    }
}

macro_rules! block_shift {
    ($ptr: expr) => {
        unsafe { (($ptr) as *mut std::ffi::c_void).add(mem::size_of::<Block>()) }
    };
}

macro_rules! block_unshift {
    ($ptr: expr) => {
        unsafe { (($ptr) as *mut std::ffi::c_void).sub(mem::size_of::<Block>()) }
    };
}

macro_rules! heap_shift {
    ($ptr: expr) => {
        unsafe { (($ptr) as *mut std::ffi::c_void).add(mem::size_of::<Heap>()) }
    };
}

macro_rules! heap_unshift {
    ($ptr: expr) => {
        unsafe { (($ptr) as *mut std::ffi::c_void).sub(mem::size_of::<Heap>()) }
    };
}

static mut HEAP_ANCHOR: *mut Heap = 0 as *mut Heap;

fn create_heap(size: usize) -> *const Heap {
    let header = Heap::new(size);
    let ptr = mem_map(header.total_size).unwrap() as *mut Heap;
    unsafe {
        ptr.write(header);
    }
    ptr
}

fn get_last_block(block_ptr: *const Block) -> *mut Block {
    let mut last_block = block_ptr;
    unsafe {
        while last_block.read().next != 0 as *const Block {
            last_block = last_block.read().next
        }
    }
    last_block as *mut Block
}

fn align(to: usize, from: usize) -> usize {
    return (from + to - 1) & !(to - 1);
}

fn get_free_block(size: usize) -> Option<*mut Block> {
    unsafe {
        let mut curr_block = heap_shift!(HEAP_ANCHOR) as *mut Block;
        loop {
            if curr_block.read().free && curr_block.read().data_size >= size {
                return Some(curr_block);
            }

            if curr_block.read().next.is_null() {
                break;
            }

            curr_block = curr_block.read().next as *mut Block;
        }
        None
    }
}

fn get_heap(size: usize) -> Option<*mut Heap> {
    let s = size;
    if unsafe { HEAP_ANCHOR }.is_null() {
        unsafe {
            HEAP_ANCHOR = create_heap(size) as *mut Heap;
        }
    }
    let heap_group: HeapGroup = s.into();
    let mut first_heap = unsafe { HEAP_ANCHOR };
    loop {
        if discriminant(&unsafe { first_heap.read() }.group) == discriminant(&heap_group)
            && unsafe { first_heap.read() }.free_size >= s + Block::size()
        {
            break Some(first_heap);
        }
        first_heap = unsafe { first_heap.read() }.next as *mut Heap;
        if first_heap.is_null() {
            break None;
        }
    }
}

fn malloc(size: usize) -> *const c_void {
    let size = align(8, size);
    if size > SMALL_HEAP_ALLOCATION_SIZE {
        let ptr = mem_map(size + Block::size()).unwrap() as *mut Block;
        unsafe { (*ptr).data_size = size };
        println!("{:?}", unsafe { *ptr });
        return block_shift!(ptr) as *const c_void;
    }

    let suitable_heap = match get_heap(size) {
        Some(h) => h,
        None => {
            let new_heap = create_heap(size) as *mut Heap;
            unsafe {
                (*new_heap).next = HEAP_ANCHOR;
                (*HEAP_ANCHOR).previous = new_heap;
                HEAP_ANCHOR = new_heap as *mut Heap;
            }
            new_heap
        }
    };
    let mut block_header = Block::new(size);
    if unsafe { suitable_heap.read().block_count } == 0 {
        let last_block = heap_shift!(suitable_heap) as *mut Block;
        unsafe {
            (*suitable_heap).block_count += 1;
            (*suitable_heap).free_size -= block_header.data_size + Block::size();
            last_block.write(block_header);
        }
        block_shift!(last_block)
    } else {
        if let Some(free_block) = get_free_block(size) {
            if unsafe { free_block.read().data_size } == size {
                return free_block as *const c_void;
            } else {
                unsafe {
                    let block2 = block_shift!(free_block).add(size) as *mut Block;
                    (*block2).free = true;
                    (*block2).data_size = free_block.read().data_size - Block::size() - size;
                    (*block2).next = null();
                    (*block2).previous = free_block;

                    (*free_block).data_size = size;
                    (*free_block).free = false;
                    (*free_block).next = block2;

                    (*suitable_heap).block_count += 1;
                }
                return block_shift!(free_block) as *const c_void;
            }
        } else {
            let last_block = get_last_block(heap_shift!(suitable_heap) as *mut Block);
            unsafe {
                let new_block =
                    block_shift!(last_block).add(last_block.read().data_size) as *mut Block;
                block_header.previous = last_block;
                (*last_block).next = new_block as *const Block;
                (*suitable_heap).block_count += 1;
                (*suitable_heap).free_size -= block_header.data_size + Block::size();
                new_block.write(block_header);
                block_shift!(new_block)
            }
        }
    }
}

fn print_heap() {
    unsafe {
        let mut current_heap = HEAP_ANCHOR;

        while !current_heap.is_null() {
            println!("==== Heap ====\n {:?}", *current_heap);
            println!("====== blocks ====");

            let mut curr_block = heap_shift!(current_heap) as *mut Block;
            while !curr_block.is_null() {
                println!("{:?}", curr_block.read());
                curr_block = curr_block.read().next as *mut Block;
            }
            current_heap = (*current_heap).next
        }
    }
}

fn parent_heap(mut block: *const c_void) -> Option<*mut Heap> {
    let mut curr_heap = unsafe { HEAP_ANCHOR };
    while !curr_heap.is_null() {
        let mut curr = heap_shift!(curr_heap) as *mut Block;
        while unsafe { curr_heap.read() }.block_count > 0 && !curr.is_null() {
            let ptr = block_shift!(curr);
            if ptr == block as *mut c_void {
                return Some(curr_heap);
            }
            curr = unsafe { *curr }.next as *mut Block;
        }
        curr_heap = unsafe { (*curr_heap).next }
    }
    None
}

fn merge_right(block: *mut Block, heap: *mut Heap) {
    unsafe {
        if !(*block).next.is_null() {
            if (*(*block).next).free {
                (*block).data_size += (*(*block).next).data_size + Block::size();
                let nxt = (*(*block).next).next as *mut Block;
                (*block).next = nxt;
                if !nxt.is_null() {
                    (*nxt).previous = block;
                }
                (*heap).block_count -= 1;
            }
        }
    }
}

fn merge_left(block: *mut Block, heap: *mut Heap) {
    unsafe {
        if !(*block).previous.is_null() {
            if (*(*block).previous).free {
                let prev_ptr = (*block).previous as *mut Block;
                let next_ptr = (*block).next as *mut Block;
                (*prev_ptr).next = next_ptr;
                (*prev_ptr).data_size += (*block).data_size + Block::size();

                if !next_ptr.is_null() {
                    (*next_ptr).previous = prev_ptr;
                }

                (*heap).block_count -= 1;
            }
        }
        if heap.read().block_count == 1 {
            let block = heap_shift!(heap) as *const Block;
            if block.read().free {
                if !(*heap).next.is_null() {
                    if !(*heap).previous.is_null() {
                        (*(*heap).previous).next = (*heap).next;
                    }
                }
                if !(*heap).previous.is_null() {
                    if !(*heap).next.is_null() {
                        (*(*heap).next).previous = (*heap).previous;
                    }
                }
                if heap == HEAP_ANCHOR {
                    HEAP_ANCHOR = (*heap).next
                }
                mem_unmap(heap as *const c_void, heap.read().total_size).unwrap();
            }
        }
    }
}

fn free(ptr: *const c_void) {
    let block_ptr = block_unshift!(ptr) as *mut Block;
    if unsafe { block_ptr.read().data_size > SMALL_HEAP_ALLOCATION_SIZE } {
        mem_unmap(block_ptr as *const c_void, unsafe {
            block_ptr.read().data_size + Block::size()
        })
        .unwrap();
        return;
    }

    let heap = match parent_heap(ptr) {
        Some(h) => h,
        None => panic!("invalid pointer"),
    };
    let block = block_unshift!(ptr) as *mut Block;
    if unsafe { block.read().free } {
        panic!("double free detected");
    }
    unsafe {
        (*block).free = true;
        (*heap).free_size += (*block).data_size + Block::size();
    }
    merge_right(block, heap);
    merge_left(block, heap);
}

fn main() {
    let ptr = malloc(5);
    let ptr = malloc(150000);
    free(ptr as *mut c_void);
    //free(ptr as *mut c_void);

    //let ptr1 = malloc(10);
    //let ptr2 = malloc(10);
    //let ptr3 = malloc(10);
    //let ptr4 = malloc(10);
    //let ptr5 = malloc(10);

    //let ptr3 = malloc(10);
    //let ptr4 = malloc(10);

    //free(ptr4 as *mut c_void);
    //free(ptr3 as *mut c_void);
    //free(ptr5 as *mut c_void);
    //free(ptr2 as *mut c_void);
    //free(ptr1 as *mut c_void);

    //println!("{:?}", align(16, 15));
    //let ptr = unsafe{block_unshift!(ptr3)} as *const Block;
    //println!("*{:?}", unsafe{HEAP_ANCHOR.read()});

    print_heap();
}

// 7461875
// 9250373

