use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use speedy::{Readable, Writable};

use crate::{AddressableAsset, Asset, Shader};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Readable, Writable, Serialize, Deserialize)]
pub enum BlendFactor {
    #[serde(rename = "zero")]
    Zero,
    #[serde(rename = "one")]
    One,
    #[serde(rename = "src_color")]
    SrcColor,
    #[serde(rename = "one_minus_src_color")]
    OneMinusSrcColor,
    #[serde(rename = "dst_color")]
    DstColor,
    #[serde(rename = "one_minus_dst_color")]
    OneMinusDstColor,
    #[serde(rename = "src_alpha")]
    SrcAlpha,
    #[serde(rename = "one_minus_src_alpha")]
    OneMinusSrcAlpha,
    #[serde(rename = "dst_alpha")]
    DstAlpha,
    #[serde(rename = "one_minus_dst_alpha")]
    OneMinusDstAlpha,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Readable, Writable, Serialize, Deserialize)]
pub enum BlendOp {
    #[serde(rename = "add")]
    Add,
    #[serde(rename = "subtract")]
    Subtract,
    #[serde(rename = "reverse_subtract")]
    ReverseSubtract,
    #[serde(rename = "min")]
    Min,
    #[serde(rename = "max")]
    Max,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Readable, Writable, Serialize, Deserialize)]
pub enum CompareOp {
    #[serde(rename = "never")]
    Never,
    #[serde(rename = "less")]
    Less,
    #[serde(rename = "equal")]
    Equal,
    #[serde(rename = "less_or_equal")]
    LessOrEqual,
    #[serde(rename = "greater")]
    Greater,
    #[serde(rename = "not_equal")]
    NotEqual,
    #[serde(rename = "greater_or_equal")]
    GreatedOrEqual,
    #[serde(rename = "always")]
    Always,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Readable, Writable, Serialize, Deserialize)]
pub enum CullMode {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "front")]
    Front,
    #[serde(rename = "back")]
    Back,
    #[serde(rename = "both")]
    FrontAndBack,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Readable, Writable, Serialize, Deserialize)]
pub enum FrontFace {
    #[serde(rename = "cw")]
    Clockwise,
    #[serde(rename = "ccw")]
    CounterClockwise,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Readable, Writable, Serialize, Deserialize)]
pub struct BlendDesc {
    pub src: BlendFactor,
    pub dst: BlendFactor,
    pub op: BlendOp,
}

#[derive(Debug, Clone, Readable, Writable)]
pub struct Pipeline {
    pub shaders: Vec<Shader>,
    pub blend: Option<(BlendDesc, BlendDesc)>,
    pub depth_test: Option<CompareOp>,
    pub depth_write: bool,
    pub cull: Option<(CullMode, FrontFace)>,
}

#[derive(Debug, Clone, Readable, Writable)]
pub struct EffectAsset(HashMap<String, Pipeline>);

impl AddressableAsset for EffectAsset {
    const TYPE_ID: uuid::Uuid = uuid::uuid!("8eb9f260-5912-46a3-8dc6-fb4fd30ab2c5");
}

impl Asset for EffectAsset {
    fn serialize<W: std::io::prelude::Write>(&self, w: &mut W) -> std::io::Result<()> {
        Ok(self.write_to_stream(w)?)
    }

    fn deserialize<R: std::io::prelude::Read>(r: &mut R) -> std::io::Result<Self> {
        Ok(Self::read_from_stream_unbuffered(r)?)
    }

    fn collect_depenencies(&self, _dependencies: &mut std::collections::HashSet<crate::AssetRef>) {}
}
