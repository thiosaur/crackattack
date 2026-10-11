//! Ring buffer for dynamic uniform buffers
//!
//! WebGPU requires aligned dynamic offsets for uniform buffers.
//! This module provides a ring buffer that handles alignment automatically.

use crate::renderer::gpu::uniforms::UBO_ALIGNMENT;

/// Ring buffer for uniform data with dynamic offsets
///
/// WebGPU requires dynamic uniform buffer offsets to be aligned.
/// This ring buffer tracks the current write offset and ensures
/// proper alignment for each write.
pub struct UniformRingBuffer {
    buffer: wgpu::Buffer,
    capacity: u32,
    current_offset: u32,
    alignment: u32,
}

impl UniformRingBuffer {
    /// Create a new ring buffer
    ///
    /// # Arguments
    /// * `device` - The wgpu device
    /// * `slot_count` - Number of slots (each slot is `alignment` bytes)
    /// * `alignment` - Alignment in bytes (typically UBO_ALIGNMENT = 512)
    /// * `label` - Debug label for the buffer
    pub fn new(device: &wgpu::Device, slot_count: usize, alignment: u64, label: &str) -> Self {
        let size = slot_count as u64 * alignment;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            buffer,
            capacity: slot_count as u32,
            current_offset: 0,
            alignment: alignment as u32,
        }
    }

    /// Create a new ring buffer with default UBO_ALIGNMENT (512)
    pub fn with_default_alignment(device: &wgpu::Device, slot_count: usize, label: &str) -> Self {
        Self::new(device, slot_count, UBO_ALIGNMENT, label)
    }

    /// Reset the write offset to 0 (call at frame start)
    pub fn reset(&mut self) {
        self.current_offset = 0;
    }

    /// Write data and return the offset for dynamic binding
    ///
    /// # Arguments
    /// * `queue` - The wgpu queue
    /// * `data` - Data to write (must be Pod)
    ///
    /// # Returns
    /// The byte offset where data was written (for set_bind_group dynamic offset)
    ///
    /// # Panics
    /// Panics if ring buffer is full.
    ///
    /// The buffer cannot grow, because a bind group holds it. The dispatcher
    /// writes once per frame into 16 slots. If the render graph grows, raise the
    /// capacity at creation instead of relying on the overflow assert.
    pub fn write<T: bytemuck::Pod>(&mut self, queue: &wgpu::Queue, data: &T) -> u32 {
        let slot = self.current_offset / self.alignment;
        assert!(
            slot < self.capacity,
            "UniformRingBuffer overflow: {} slots used, capacity {}",
            slot,
            self.capacity
        );

        let offset = self.current_offset;
        queue.write_buffer(&self.buffer, offset as u64, bytemuck::bytes_of(data));
        self.current_offset += self.alignment;
        offset
    }

    /// Get the underlying buffer
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}
