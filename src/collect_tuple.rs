pub struct OrPanic<Tuple>(pub Tuple);

macro_rules! implement {
    (@asitem $x:item) => ($x);
    (@impl $first_ty:tt $first:tt $($ty:tt $x:tt)*) => {implement!{@asitem
        impl<$first_ty> ::std::iter::FromIterator<$first_ty> for OrPanic<($first_ty, $($ty,)*)> {
            fn from_iter<I>(iter: I) -> Self where I: IntoIterator<Item=$first_ty> {
                let mut iter = iter.into_iter();
                let $first = iter.next().unwrap();
                $( let $x = iter.next().unwrap(); )*
                    if iter.next().is_some() { panic!("too many elements"); }
                OrPanic(($first, $($x,)* ))
            }
        }}

        implement!{@impl $($ty $x)*}
    };
    (@impl) => {};

    (@do $ty:tt ($($done:tt)*) $x:tt $($rest:tt)*) => {implement!{@do $ty ($($done)* $ty $x) $($rest)*}};
    (@do $ty:tt ($($done:tt)*)) => {implement!{@impl $($done)*}};
    ($($x:tt)*) => {implement!{@do T () $($x)*}};
}

implement! {a b c d e f g h i j k l m n o p q r s t u v w x y z}
