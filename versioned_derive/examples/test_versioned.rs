use serde::{Serialize, Serializer};
use serde_json;

use versioned_derive::{Versioned};
use versioned::{Versioned, Atomic, update};

fn serialize_as_hex<S>(value: &Option<i32>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(v) => serializer.serialize_str(&format!("{:x}", v)),
        None => serializer.serialize_none(),
    }
}


#[derive(Versioned)]
#[serde(tag = "bongo", rename_all="snake_case")]
enum MyEnum {
    // error: unknown serde variant attribute `transparent`
    // #[serde(transparent)]
    E1,

    E2 {
        #[serde(serialize_with = "serialize_as_hex")]
        field1: i32,
        field2: f64,
    },

    // #[serde(skip_serializing)]
    // E3(i32, f64),

    E4 {
        // #[serde(skip_serializing_if = "Option::is_none")]
        // field_opt: Option<i32>,
        field_opt: i32,

        #[serde(rename = "field_three")]
        field3: String,
    },

    // #[serde(with = "date_as_string")]
    // E5(String),
}

#[derive(Serialize, Clone)]
struct AtomicStruct {
    my_int: i32,
    my_float: f64,
}

impl Atomic for AtomicStruct {}

#[derive(Versioned)]
struct MyStruct {
    my_atomic: AtomicStruct,

    my_int: i32,
    my_float: f64,
    
    #[serde(flatten)]
    my_enum: MyEnum,

    #[serde(serialize_with = "serialize_as_hex")]
    my_hex: i32,
    
    #[serde(skip)]
    my_secret: String,

    // #[serde(default)]
    // my_default: Option<i32>,
}

fn main() {
    let mut my_struct = Versioned::new(MyStruct {
        my_atomic: AtomicStruct {
            my_int: 42,
            my_float: 3.14,
        },
        my_int: 42,
        my_float: 3.14,
        my_enum: MyEnum::E1,
        my_hex: 0x3EADBEEF,
        my_secret: "secret".to_string(),
        // my_default: Some(10),
    }, 0);

    update!(my_struct.my_atomic, AtomicStruct {
        my_int: 43,
        my_float: 42.23,
    });

    update!(my_struct.my_int, 43);
    update!(my_struct.my_enum, MyEnum::E2 { field1: 23, field2: 3.14 });
    update!(my_struct.my_enum, MyEnum::E2 { field2: _ }, 42.23);
    update!(my_struct.my_enum, MyEnum::E4 { field_opt: 10, field3: "hello".to_string() });
    update!(my_struct.my_hex, 0x2EEFCAFE);
    update!(my_struct.my_secret, "new secret".to_string());

    let delta = MyStruct::get(&my_struct, 6);
    let serialized = serde_json::to_string(&delta).unwrap();
    println!("{}", serialized);

    let delta = MyStruct::get(&my_struct, 7);
    let serialized = serde_json::to_string(&delta).unwrap();
    println!("{}", serialized);

    update!(my_struct.my_enum, MyEnum::E4 { field_opt: _ }, 11);

    let delta = MyStruct::get(&my_struct, 7);
    let serialized = serde_json::to_string(&delta).unwrap();
    println!("{}", serialized);

}
