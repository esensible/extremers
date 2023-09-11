use serde::{Serialize};
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

    let delta = MyStruct::get(my_struct, 2);
    let serialized = serde_json::to_string(&delta).unwrap();
    println!("{}", serialized);
    // generates {"my_enum":{"E2":{"field1":42,"field2":42.23}}} as expected
}

