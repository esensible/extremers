use serde::{Serialize, Deserialize};
use serde_json;

extern crate versioned;
use versioned::{Versioned, VersionedValue, DeltaType, VersionedType, update};

extern crate versioned_derive;
use versioned_derive::Versioned;

#[derive(Versioned)]
enum MyEnum {
    E1,
    E2{
        field1: i32,
        field2: f64,
    },
}

// struct Struct2 {
//     field1: i32,
// }

#[derive(Versioned)]
struct MyStruct {
    my_int: i32,
    my_float: f64,
    my_enum: MyEnum,
    // my_struct: Struct2,
}

// enum VersionedMyEnum {
//     E1,
//     E2 {
//         field1: VersionedValue<VersionedType<i32>>,
//         field2: VersionedValue<VersionedType<f64>>,
//     },
// }

// #[derive(Serialize, Deserialize)]
// #[serde(tag = "MyEnum")]
// enum DeltaMyEnum {
//     E1,
//     E2 {
//         #[serde(skip_serializing_if = "Option::is_none")]
//         field1: DeltaType<i32>,
//         #[serde(skip_serializing_if = "Option::is_none")]
//         field2: DeltaType<f64>,
//     },
// }

// impl Versioned for MyEnum {
//     type Value = VersionedMyEnum;
//     type Delta = DeltaMyEnum;

//     fn new(value: Self, version: usize) -> VersionedValue<Self::Value> {
//         let value = match value {
//             MyEnum::E1 => Self::Value::E1,
//             MyEnum::E2 { field1, field2 } => Self::Value::E2 {
//                 field1: Versioned::new(field1, version),
//                 field2: Versioned::new(field2, version),
//             },
//         };
//         VersionedValue { value, version }
//     }

//     fn get(value: VersionedValue<Self::Value>, version: usize) -> DeltaMyEnum {
//         match value.value {
//             VersionedMyEnum::E1 => DeltaMyEnum::E1,
//             VersionedMyEnum::E2 { field1, field2 } => DeltaMyEnum::E2 {
//                 field1: i32::get(field1, version),
//                 field2: f64::get(field2, version),
//             },
//         }
//     }

// }


// struct VersionedMyStruct {
//     my_int: VersionedValue<VersionedType<i32>>,
//     my_float: VersionedValue<VersionedType<f64>>,
//     my_enum: VersionedValue<VersionedType<MyEnum>>,
//     // my_struct: VersionedValue<VersionedType<Struct2>>,
// }

// // #[derive(Serialize)]
// struct DeltaMyStruct {
//     // #[serde(skip_serializing_if = "Option::is_none")]
//     my_int: DeltaType<i32>,
//     // #[serde(skip_serializing_if = "Option::is_none")]
//     my_float: DeltaType<f64>,
//     // #[serde(flatten)]
//     my_enum: DeltaType<MyEnum>,
//     // #[serde(skip_serializing_if = "Option::is_none")]
//     // my_struct: DeltaType<Struct2>,
// }

// impl Versioned for MyStruct {
//     type Value = VersionedMyStruct;
//     type Delta = DeltaMyStruct;

//     fn new(value: Self, version: usize) -> VersionedValue<Self::Value> {
//         VersionedValue {
//             value: Self::Value {
//                 my_int: Versioned::new(value.my_int, version),
//                 my_float: Versioned::new(value.my_float, version),
//                 my_enum: Versioned::new(value.my_enum, version),
//                 // my_struct: Versioned::new(value.my_struct, version),
//             },
//             version: version,
//         }
//     }

//     fn get(value: VersionedValue<Self::Value>, version: usize) -> DeltaMyStruct {
//         DeltaMyStruct {
//             my_int: i32::get(value.value.my_int, version),
//             my_float: f64::get(value.value.my_float, version),
//             my_enum: MyEnum::get(value.value.my_enum, version),
//             // my_struct: Struct2::get(value.value.my_struct, version),
//         }
//     }
// }

// examples/basic_example.rs

fn main() {
    let mut my_struct = Versioned::new(MyStruct {
        my_int: 42,
        my_float: 3.14,
        my_enum: MyEnum::E1,
        // my_struct: Struct2 {
        //     field1: 23,
        // },
    }, 0);

    update!(my_struct.my_int, 43);
    update!(my_struct.my_enum, MyEnum::E2{field1: 23, field2: 3.14});
    update!(my_struct.my_enum, MyEnum::E2{field1:_}, 42);
    update!(my_struct.my_enum, MyEnum::E2{field2:_}, 42.23);
    // update!(MyStruct.my_struct, Struct2{field1: 24});

    let delta = MyStruct::get(my_struct, 4);
    let serialized = serde_json::to_string(&delta).unwrap();
    println!("{}", serialized);

    // let delta2 = MyStruct::get(my_struct, 3);
    // let serialized2 = serde_json::to_string(&delta2).unwrap();
    // println!("{}", serialized2);
}

