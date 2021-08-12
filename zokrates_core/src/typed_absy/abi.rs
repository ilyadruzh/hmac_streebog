use crate::typed_absy::{Signature, Type};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct AbiInput {
    pub name: String,
    pub public: bool,
    #[serde(flatten)]
    pub ty: Type,
}

pub type AbiOutput = Type;

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Abi {
    pub inputs: Vec<AbiInput>,
    pub outputs: Vec<AbiOutput>,
}

impl Abi {
    pub fn signature(&self) -> Signature {
        Signature {
            inputs: self.inputs.iter().map(|i| i.ty.clone()).collect(),
            outputs: self.outputs.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed_absy::types::{ArrayType, FunctionKey, StructMember, StructType};
    use crate::typed_absy::{
        Parameter, Type, TypedFunction, TypedFunctionSymbol, TypedModule, TypedProgram, UBitwidth,
        Variable,
    };
    use std::collections::HashMap;
    use zokrates_field::Bn128Field;

    #[test]
    fn generate_abi_from_typed_ast() {
        let mut functions = HashMap::new();
        functions.insert(
            FunctionKey::with_id("main"),
            TypedFunctionSymbol::Here(TypedFunction {
                arguments: vec![
                    Parameter {
                        id: Variable::field_element("a"),
                        private: true,
                    },
                    Parameter {
                        id: Variable::boolean("b"),
                        private: false,
                    },
                ],
                statements: vec![],
                signature: Signature::new()
                    .inputs(vec![Type::FieldElement, Type::Boolean])
                    .outputs(vec![Type::FieldElement]),
            }),
        );

        let mut modules = HashMap::new();
        modules.insert("main".into(), TypedModule { functions });

        let typed_ast: TypedProgram<Bn128Field> = TypedProgram {
            main: "main".into(),
            modules,
        };

        let abi: Abi = typed_ast.abi();
        let expected_abi = Abi {
            inputs: vec![
                AbiInput {
                    name: String::from("a"),
                    public: false,
                    ty: Type::FieldElement,
                },
                AbiInput {
                    name: String::from("b"),
                    public: true,
                    ty: Type::Boolean,
                },
            ],
            outputs: vec![Type::FieldElement],
        };

        assert_eq!(expected_abi, abi);
    }

    #[test]
    fn serialize_empty() {
        let abi: Abi = Abi {
            inputs: vec![],
            outputs: vec![],
        };

        let json = serde_json::to_string(&abi).unwrap();
        assert_eq!(&json, r#"{"inputs":[],"outputs":[]}"#);
        let de_abi: Abi = serde_json::from_str(json.as_ref()).unwrap();
        assert_eq!(de_abi, abi);
    }

    #[test]
    fn serialize_field() {
        let abi: Abi = Abi {
            inputs: vec![
                AbiInput {
                    name: String::from("a"),
                    public: true,
                    ty: Type::FieldElement,
                },
                AbiInput {
                    name: String::from("b"),
                    public: true,
                    ty: Type::FieldElement,
                },
            ],
            outputs: vec![Type::FieldElement],
        };

        let json = serde_json::to_string_pretty(&abi).unwrap();
        assert_eq!(
            &json,
            r#"{
  "inputs": [
    {
      "name": "a",
      "public": true,
      "type": "field"
    },
    {
      "name": "b",
      "public": true,
      "type": "field"
    }
  ],
  "outputs": [
    {
      "type": "field"
    }
  ]
}"#
        );

        let de_abi: Abi = serde_json::from_str(json.as_ref()).unwrap();
        assert_eq!(de_abi, abi);
    }

    #[test]
    fn serialize_uints() {
        let abi: Abi = Abi {
            inputs: vec![
                AbiInput {
                    name: String::from("a"),
                    public: true,
                    ty: Type::Uint(UBitwidth::B8),
                },
                AbiInput {
                    name: String::from("b"),
                    public: true,
                    ty: Type::Uint(UBitwidth::B16),
                },
                AbiInput {
                    name: String::from("c"),
                    public: true,
                    ty: Type::Uint(UBitwidth::B32),
                },
            ],
            outputs: vec![],
        };

        let json = serde_json::to_string_pretty(&abi).unwrap();
        assert_eq!(
            &json,
            r#"{
  "inputs": [
    {
      "name": "a",
      "public": true,
      "type": "u8"
    },
    {
      "name": "b",
      "public": true,
      "type": "u16"
    },
    {
      "name": "c",
      "public": true,
      "type": "u32"
    }
  ],
  "outputs": []
}"#
        );

        let de_abi: Abi = serde_json::from_str(json.as_ref()).unwrap();
        assert_eq!(de_abi, abi);
    }

    #[test]
    fn serialize_struct() {
        let abi: Abi = Abi {
            inputs: vec![AbiInput {
                name: String::from("foo"),
                public: true,
                ty: Type::Struct(StructType::new(
                    "".into(),
                    "Foo".into(),
                    vec![
                        StructMember::new(String::from("a"), Type::FieldElement),
                        StructMember::new(String::from("b"), Type::Boolean),
                    ],
                )),
            }],
            outputs: vec![Type::Struct(StructType::new(
                "".into(),
                "Foo".into(),
                vec![
                    StructMember::new(String::from("a"), Type::FieldElement),
                    StructMember::new(String::from("b"), Type::Boolean),
                ],
            ))],
        };

        let json = serde_json::to_string_pretty(&abi).unwrap();
        assert_eq!(
            &json,
            r#"{
  "inputs": [
    {
      "name": "foo",
      "public": true,
      "type": "struct",
      "components": {
        "name": "Foo",
        "members": [
          {
            "name": "a",
            "type": "field"
          },
          {
            "name": "b",
            "type": "bool"
          }
        ]
      }
    }
  ],
  "outputs": [
    {
      "type": "struct",
      "components": {
        "name": "Foo",
        "members": [
          {
            "name": "a",
            "type": "field"
          },
          {
            "name": "b",
            "type": "bool"
          }
        ]
      }
    }
  ]
}"#
        );

        let de_abi: Abi = serde_json::from_str(json.as_ref()).unwrap();
        assert_eq!(de_abi, abi);
    }

    #[test]
    fn serialize_nested_struct() {
        let abi: Abi = Abi {
            inputs: vec![AbiInput {
                name: String::from("foo"),
                public: true,
                ty: Type::Struct(StructType::new(
                    "".into(),
                    "Foo".into(),
                    vec![StructMember::new(
                        String::from("bar"),
                        Type::Struct(StructType::new(
                            "".into(),
                            "Bar".into(),
                            vec![
                                StructMember::new(String::from("a"), Type::FieldElement),
                                StructMember::new(String::from("b"), Type::FieldElement),
                            ],
                        )),
                    )],
                )),
            }],
            outputs: vec![],
        };

        let json = serde_json::to_string_pretty(&abi).unwrap();
        assert_eq!(
            &json,
            r#"{
  "inputs": [
    {
      "name": "foo",
      "public": true,
      "type": "struct",
      "components": {
        "name": "Foo",
        "members": [
          {
            "name": "bar",
            "type": "struct",
            "components": {
              "name": "Bar",
              "members": [
                {
                  "name": "a",
                  "type": "field"
                },
                {
                  "name": "b",
                  "type": "field"
                }
              ]
            }
          }
        ]
      }
    }
  ],
  "outputs": []
}"#
        );

        let de_abi: Abi = serde_json::from_str(json.as_ref()).unwrap();
        assert_eq!(de_abi, abi);
    }

    #[test]
    fn serialize_struct_array() {
        let abi: Abi = Abi {
            inputs: vec![AbiInput {
                name: String::from("a"),
                public: false,
                ty: Type::Array(ArrayType::new(
                    Type::Struct(StructType::new(
                        "".into(),
                        "Foo".into(),
                        vec![
                            StructMember::new(String::from("b"), Type::FieldElement),
                            StructMember::new(String::from("c"), Type::Boolean),
                        ],
                    )),
                    2,
                )),
            }],
            outputs: vec![Type::Boolean],
        };

        let json = serde_json::to_string_pretty(&abi).unwrap();
        assert_eq!(
            &json,
            r#"{
  "inputs": [
    {
      "name": "a",
      "public": false,
      "type": "array",
      "components": {
        "size": 2,
        "type": "struct",
        "components": {
          "name": "Foo",
          "members": [
            {
              "name": "b",
              "type": "field"
            },
            {
              "name": "c",
              "type": "bool"
            }
          ]
        }
      }
    }
  ],
  "outputs": [
    {
      "type": "bool"
    }
  ]
}"#
        );

        let de_abi: Abi = serde_json::from_str(json.as_ref()).unwrap();
        assert_eq!(de_abi, abi);
    }

    #[test]
    fn serialize_multi_dimensional_array() {
        let abi: Abi = Abi {
            inputs: vec![AbiInput {
                name: String::from("a"),
                public: false,
                ty: Type::Array(ArrayType::new(
                    Type::Array(ArrayType::new(Type::FieldElement, 2)),
                    2,
                )),
            }],
            outputs: vec![Type::FieldElement],
        };

        let json = serde_json::to_string_pretty(&abi).unwrap();
        assert_eq!(
            &json,
            r#"{
  "inputs": [
    {
      "name": "a",
      "public": false,
      "type": "array",
      "components": {
        "size": 2,
        "type": "array",
        "components": {
          "size": 2,
          "type": "field"
        }
      }
    }
  ],
  "outputs": [
    {
      "type": "field"
    }
  ]
}"#
        );

        let de_abi: Abi = serde_json::from_str(json.as_ref()).unwrap();
        assert_eq!(de_abi, abi);
    }
}
