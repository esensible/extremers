// mod race;
// mod engine_traits;
// mod engine_context;

// use crossbeam_channel::unbounded;
// use engine_context::EngineContext;
// use race::Race;

// fn main() {
//     let ctx = EngineContext::default();
    
//     let (notify_sender, notify_receiver) = unbounded::<String>();

//     let notify_fn = move |event: String| {
//         notify_sender.send(event).unwrap();
//     };

//     ctx.set_engine(Race::default(), Box::new(notify_fn));

//     assert_eq!(ctx.handle_event("\"hello\""), Ok("Event scheduled"));

// }

use versioned_derive::Delta;
use versioned::{Atomic, DeltaTrait};
use serde_derive::Serialize;
use serde_json as json;

#[derive(Delta, Clone, PartialEq)]
enum E {
    A,
    B{
        b1: i32, b2: i32
    }
}


#[derive(Delta, Clone, PartialEq)]
struct A {
    a1: i32,
    a2: i32,
    a3: E,
}

fn main () {
    let mut a = A { a1: 1, a2: 2, a3: E::A };
    let delta = <A as DeltaTrait>::delta(&a, &a);

    println!("Delta: {:?}", json::to_string(&delta).unwrap());

    let old_a = a.clone();
    a.a2 += 1;
    a.a3 = E::B{b1: 3, b2: 4};
    let delta = <A as DeltaTrait>::delta(&a, &old_a);

    println!("Delta: {:?}", json::to_string(&delta).unwrap());

    let old_a = a.clone();
    a.a1 += 1;
    let delta = A::delta(&a, &old_a);
    println!("Delta: {:?}", json::to_string(&delta).unwrap());

    let old_a = a.clone();
    if let E::B{b1, b2} = &mut a.a3 {
        *b1 += 1;
        // *b2 += 1;
    }
    let delta = A::delta(&a, &old_a);
    println!("Delta: {:?}", json::to_string(&delta).unwrap());

}
