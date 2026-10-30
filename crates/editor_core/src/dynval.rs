//! Dynamic values: what core carries for a domain without knowing its types (design §6.2). A domain wraps its own
//! action / property types in [`Payload`] / [`PropKey`]; equality is by value so undo merging and redraw keys work.

use std::any::Any;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;

/// Object-safe value equality over `Any` types.
pub trait DynPayload: Any + Debug + Send + Sync {
    fn dyn_eq(&self, other: &dyn DynPayload) -> bool;
}

impl<T: Any + Debug + PartialEq + Send + Sync> DynPayload for T {
    fn dyn_eq(&self, other: &dyn DynPayload) -> bool {
        (other as &dyn Any).downcast_ref::<T>().is_some_and(|o| o == self)
    }
}

/// A type-erased domain value, compared by value.
#[derive(Clone)]
pub struct Payload(Arc<dyn DynPayload>);

impl Payload {
    pub fn new<T: DynPayload>(t: T) -> Payload {
        Payload(Arc::new(t))
    }

    pub fn downcast<T: Any>(&self) -> Option<&T> {
        (&*self.0 as &dyn Any).downcast_ref::<T>()
    }
}

impl PartialEq for Payload {
    fn eq(&self, other: &Payload) -> bool {
        self.0.dyn_eq(&*other.0)
    }
}

impl Debug for Payload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A domain's property target (which field of which object a widget edits).
pub trait PropTarget: DynPayload {
    /// Longest text a text field for this target accepts.
    fn max_chars(&self) -> Option<usize> {
        None
    }
}

/// A type-erased [`PropTarget`], compared by value.
#[derive(Clone)]
pub struct PropKey(Arc<dyn PropTarget>);

impl PropKey {
    pub fn new<T: PropTarget>(t: T) -> PropKey {
        PropKey(Arc::new(t))
    }

    pub fn downcast<T: Any>(&self) -> Option<&T> {
        (&*self.0 as &dyn Any).downcast_ref::<T>()
    }

    pub fn max_chars(&self) -> Option<usize> {
        self.0.max_chars()
    }
}

/// The key of a widget that edits nothing.
#[derive(Debug, PartialEq)]
pub struct NoTarget;

impl PropTarget for NoTarget {}

impl Default for PropKey {
    fn default() -> PropKey {
        PropKey::new(NoTarget)
    }
}

impl<T: PropTarget> From<T> for PropKey {
    fn from(t: T) -> PropKey {
        PropKey::new(t)
    }
}

impl PartialEq for PropKey {
    fn eq(&self, other: &PropKey) -> bool {
        self.0.dyn_eq(&*other.0)
    }
}

/// A key equals a target of its own type with equal contents (`field.target == CurveProp::Range`).
impl<T: PropTarget + PartialEq> PartialEq<T> for PropKey {
    fn eq(&self, other: &T) -> bool {
        self.downcast::<T>() == Some(other)
    }
}

impl Debug for PropKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A kit's property target `P` tagged with the domain type `D` its editor runs for (design §14.4): a kit document of
/// domain `D` (`curve_kit`'s `CurveDoc<D>`, `texture_kit`'s `TextureDoc<D>`) takes only `KitProp<D, P>`, so a key
/// meant for another domain's tab of the same kit is not applied to it. Compared by value like any target. `P` itself
/// is no [`PropTarget`], so an untagged key cannot be built by mistake; its text fields have no length limit.
pub struct KitProp<D, P>(pub P, PhantomData<fn() -> D>);

impl<D, P> KitProp<D, P> {
    pub fn new(prop: P) -> KitProp<D, P> {
        KitProp(prop, PhantomData)
    }
}

impl<D, P: Clone> Clone for KitProp<D, P> {
    fn clone(&self) -> Self {
        KitProp::new(self.0.clone())
    }
}

impl<D, P: PartialEq> PartialEq for KitProp<D, P> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<D, P: Debug> Debug for KitProp<D, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} for {}", self.0, std::any::type_name::<D>())
    }
}

impl<D: 'static, P: Any + Debug + PartialEq + Send + Sync> PropTarget for KitProp<D, P> {}

/// The core widget value; a domain converts it to its own value type.
#[derive(Debug, Clone, PartialEq)]
pub enum PropValue {
    Float(f32),
    Float2([f32; 2]),
    Float3([f32; 3]),
    Float4([f32; 4]),
    Color([f32; 4]),
    Bool(bool),
    Int(i32),
    Str(String),
}

impl PropValue {
    /// Float components (scalars give one component, a bool `0` / `1`, a string `0`).
    pub fn floats(&self) -> Vec<f32> {
        match self {
            PropValue::Float(v) => vec![*v],
            PropValue::Float2(v) => v.to_vec(),
            PropValue::Float3(v) => v.to_vec(),
            PropValue::Float4(v) | PropValue::Color(v) => v.to_vec(),
            PropValue::Int(v) => vec![*v as f32],
            PropValue::Bool(b) => vec![if *b { 1.0 } else { 0.0 }],
            PropValue::Str(_) => vec![0.0],
        }
    }

    pub fn as_f32(&self) -> f32 {
        self.floats()[0]
    }

    pub fn as_bool(&self) -> bool {
        match self {
            PropValue::Bool(b) => *b,
            other => other.as_f32() != 0.0,
        }
    }

    pub fn as_i32(&self) -> i32 {
        match self {
            PropValue::Int(v) => *v,
            other => other.as_f32() as i32,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            PropValue::Str(s) => s,
            _ => "",
        }
    }

    /// What a number field editing component `comp` of `self` sets when it sends `value` (`SetComponent`): `self`
    /// with that component replaced (an `Int` rounds it; a bool or a string stays as it is). `None` when `value` is
    /// only the text the field shows (rounded, [`display_value`]) sent back on focus loss: that is not an edit.
    pub fn with_component(&self, comp: usize, value: f32) -> Option<PropValue> {
        let old = self.floats().get(comp).copied();
        if old.is_some_and(|old| old != value && display_value(old) == value) {
            return None;
        }
        let set = |v: &mut [f32]| {
            if let Some(x) = v.get_mut(comp) {
                *x = value;
            }
        };
        Some(match self.clone() {
            PropValue::Float(_) => PropValue::Float(value),
            PropValue::Int(_) => PropValue::Int(value.round() as i32),
            PropValue::Float2(mut v) => {
                set(&mut v);
                PropValue::Float2(v)
            }
            PropValue::Float3(mut v) => {
                set(&mut v);
                PropValue::Float3(v)
            }
            PropValue::Float4(mut v) => {
                set(&mut v);
                PropValue::Float4(v)
            }
            PropValue::Color(mut v) => {
                set(&mut v);
                PropValue::Color(v)
            }
            other => other,
        })
    }
}

/// Value a number field shows: Feathers prints every digit it is given (a picked color gives `0.73803073`),
/// so fields show 3 decimals like their `NumberInputPrecision`. The model keeps full precision.
pub fn display_value(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct A(i32);
    #[derive(Debug, PartialEq)]
    struct B(i32);
    impl PropTarget for A {
        fn max_chars(&self) -> Option<usize> {
            Some(self.0 as usize)
        }
    }

    #[test]
    fn payload_eq_is_by_value() {
        assert_eq!(Payload::new(A(1)), Payload::new(A(1)));
        assert_ne!(Payload::new(A(1)), Payload::new(A(2)));
        assert_ne!(Payload::new(A(1)), Payload::new(B(1)));
        assert!(Payload::new(A(1)).downcast::<B>().is_none());
        assert_eq!(Payload::new(A(1)).downcast::<A>(), Some(&A(1)));
        assert!(format!("{:?}", Payload::new(A(7))).contains("A(7)"));
    }

    #[test]
    fn prop_key_eq_and_max_chars() {
        assert_eq!(PropKey::new(A(3)), PropKey::new(A(3)));
        assert_ne!(PropKey::new(A(3)), PropKey::new(A(4)));
        assert_eq!(PropKey::new(A(3)).max_chars(), Some(3));
        assert_eq!(PropKey::default(), PropKey::default());
        assert_eq!(PropKey::default().max_chars(), None);
        assert_ne!(PropKey::default(), PropKey::new(A(0)));
        assert!(PropKey::new(A(3)).downcast::<A>().is_some());
        assert_eq!(PropKey::from(A(5)), PropKey::new(A(5)));
        assert!(PropKey::new(A(5)) == A(5) && PropKey::new(A(5)) != A(6));
        assert!(PropKey::default() != A(5));
    }

    #[test]
    fn component_edits() {
        let v = PropValue::Float3([0.1, 0.2, 0.3]);
        assert_eq!(v.with_component(1, 0.9), Some(PropValue::Float3([0.1, 0.9, 0.3])));
        assert_eq!(v.with_component(7, 0.9), Some(v.clone()), "a missing component changes nothing");
        assert_eq!(PropValue::Int(2).with_component(0, 3.6), Some(PropValue::Int(4)));
        assert_eq!(PropValue::Str("x".into()).with_component(0, 1.0), Some(PropValue::Str("x".into())));
        // A field shows 0.738 of 0.73803; focus loss sends that text back.
        assert_eq!(PropValue::Float(0.73803).with_component(0, 0.738), None);
        assert_eq!(PropValue::Float(0.73803).with_component(0, 0.5), Some(PropValue::Float(0.5)));
        assert_eq!(PropValue::Float(0.5).with_component(0, 0.5), Some(PropValue::Float(0.5)));
        assert_eq!(PropValue::Color([1.0, 0.5, 0.0, 1.0]).floats(), [1.0, 0.5, 0.0, 1.0]);
        assert!(PropValue::Bool(true).as_bool() && PropValue::Float(1.0).as_bool());
        assert_eq!(PropValue::Float(2.7).as_i32(), 2);
        assert_eq!(PropValue::Float(1.0).as_str(), "");
    }
}
