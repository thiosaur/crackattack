//! Mesh types and BuiltinMesh enum

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// Only used to dispatch garbage shells; blocks have their own dedicated buffer.
pub enum BuiltinMesh {
    // Garbage - thin (height=1)
    GarbageThinCap,
    GarbageThinMiddle,
    GarbageSmall,
    GarbageFrontal,

    // Garbage - thick (height>1)
    GarbageThickCorner,
    GarbageThickEdge,
    GarbageThickMiddle,

}
