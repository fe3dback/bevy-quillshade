//! Material Attributes category (decision 0023): pack / unpack a PBR layer into one `MaterialAttributes (PBR)` pin.

use shader_graph_core::PinType;
use shader_graph_core::attributes::{PBR_STRUCT, pbr_fields};
use shader_graph_core::registry::{PinDef, Registry};

use crate::def::{def, pin_out, sig};

const CAT: &str = "Material Attributes";

pub fn register(r: &mut Registry) {
    r.register(
        def("stdlib::MakeMaterialAttributesPbr", "MakeMaterialAttributes (PBR)", CAT)
            .kw(&["material attributes", "make", "pack", "layer", "pbr"])
            .desc("Packs PBR inputs into one Material Attributes value. Unconnected fields use fixed defaults.")
            .build(
                |_| {
                    let ins = pbr_fields().into_iter().map(|f| PinDef::new(f.id, f.label, f.ty).default(f.default));
                    sig(ins.collect(), vec![pin_out("attributes", "", PinType::MaterialAttributesPbr)])
                },
                |c| {
                    let args: Vec<String> = pbr_fields().iter().map(|f| c.input(f.id)).collect();
                    c.define("attributes", format!("{PBR_STRUCT}({})", args.join(", ")));
                },
            ),
    );

    r.register(
        def("stdlib::BreakMaterialAttributesPbr", "BreakMaterialAttributes (PBR)", CAT)
            .kw(&["material attributes", "break", "unpack", "split", "layer", "pbr"])
            .desc("Splits a Material Attributes value into its PBR fields.")
            .build(
                |_| {
                    let outs = pbr_fields().into_iter().map(|f| pin_out(f.id, f.label, f.ty));
                    sig(vec![PinDef::new("attributes", "Attributes", PinType::MaterialAttributesPbr)], outs.collect())
                },
                |c| {
                    let v = c.input("attributes");
                    for f in pbr_fields() {
                        c.define(f.id, format!("{v}.{}", f.id));
                    }
                },
            ),
    );
}
