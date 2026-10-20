//! A synthetic *structurally valid* PDF is mutated one invariant at a time.
//! Tests assert fail-closed rather than merely checking "no crash".
use super::{page_tree::validate_page_tree, Transformer, MAX_PDF_PAGES};
use crate::core::TransformerTrait;
use anyhow::Result;
use bytes::Bytes;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document as LopdfDocument, Object, ObjectId, Stream};

struct Fixture {
    pdf: LopdfDocument,
    root: ObjectId,
    first: ObjectId,
    second: ObjectId,
    resources: ObjectId,
}

impl Fixture {
    fn new() -> Result<Self> {
        let mut pdf = LopdfDocument::with_version("1.5");
        let root = pdf.new_object_id();
        let font = pdf.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let resources = pdf.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font },
        });
        let mut pages = Vec::new();
        for label in ["TreeFirstUnique", "TreeSecondUnique"] {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
                    Operation::new("Tj", vec![Object::string_literal(label)]),
                    Operation::new("ET", vec![]),
                ],
            };
            let contents = pdf.add_object(Stream::new(dictionary! {}, content.encode()?));
            pages.push(pdf.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => root,
                "Resources" => resources,
                "Contents" => contents,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            }));
        }
        pdf.objects.insert(root, Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![pages[0].into(), pages[1].into()],
            "Count" => 2,
        }));
        let catalog = pdf.add_object(dictionary! {
            "Type" => "Catalog", "Pages" => root,
        });
        pdf.trailer.set("Root", catalog);
        Ok(Self {
            pdf, root, first: pages[0], second: pages[1], resources,
        })
    }

    fn bytes(&mut self) -> Result<Bytes> {
        let mut raw = Vec::new();
        self.pdf.save_to(&mut raw)?;
        Ok(Bytes::from(raw))
    }

    fn verify_rejected(mut self, hint: &str) -> Result<()> {
        let err = Transformer::parse(&self.bytes()?)
            .expect_err("invalid /Pages tree must not yield a Complete document");
        assert!(
            err.to_string().contains(hint),
            "wrong page-tree error category: {err}"
        );
        Ok(())
    }

    fn nested(&mut self) -> Result<ObjectId> {
        let subgroup = self.pdf.add_object(dictionary! {
            "Type" => "Pages",
            "Parent" => self.root,
            "Kids" => vec![self.second.into()],
            "Count" => 1,
        });
        self.pdf.get_object_mut(self.root)?.as_dict_mut()?
            .set("Kids", vec![self.first.into(), subgroup.into()]);
        self.pdf.get_object_mut(self.second)?.as_dict_mut()?
            .set("Parent", subgroup);
        Ok(subgroup)
    }
}

#[test]
fn tree_two_pages_complete_and_ordered() -> Result<()> {
    let mut f = Fixture::new()?;
    assert_eq!(validate_page_tree(&f.pdf)?, vec![f.first, f.second]);
    let output = format!("{:?}", Transformer::parse(&f.bytes()?)?);
    assert_eq!(output.matches("TreeFirstUnique").count(), 1);
    assert_eq!(output.matches("TreeSecondUnique").count(), 1);
    assert!(output.find("TreeFirstUnique") < output.find("TreeSecondUnique"));
    Ok(())
}

#[test]
fn tree_nested_with_inherited_resources_is_complete() -> Result<()> {
    let mut f = Fixture::new()?;
    let _subtree = f.nested()?;
    // Resources and MediaBox can legitimately be inherited from /Pages.
    let root = f.pdf.get_object_mut(f.root)?.as_dict_mut()?;
    root.set("Resources", f.resources);
    root.set("MediaBox", vec![0.into(), 0.into(), 612.into(), 792.into()]);
    f.pdf.get_object_mut(f.first)?.as_dict_mut()?.remove(b"Resources");
    f.pdf.get_object_mut(f.second)?.as_dict_mut()?.remove(b"Resources");

    assert_eq!(validate_page_tree(&f.pdf)?, vec![f.first, f.second]);
    let output = format!("{:?}", Transformer::parse(&f.bytes()?)?);
    assert_eq!(output.matches("TreeFirstUnique").count(), 1);
    assert_eq!(output.matches("TreeSecondUnique").count(), 1);
    Ok(())
}

#[test]
fn tree_count_root_mismatch_rejected() -> Result<()> {
    let mut f = Fixture::new()?;
    f.pdf.get_object_mut(f.root)?.as_dict_mut()?.set("Count", 3);
    f.verify_rejected("/Count")
}

#[test]
fn tree_count_subtree_mismatch_rejected() -> Result<()> {
    let mut f = Fixture::new()?;
    let nested = f.nested()?;
    f.pdf.get_object_mut(nested)?.as_dict_mut()?.set("Count", 2);
    f.verify_rejected("/Count")
}

#[test]
fn tree_wrong_parent_rejected() -> Result<()> {
    let mut f = Fixture::new()?;
    f.pdf.get_object_mut(f.second)?.as_dict_mut()?.set("Parent", f.first);
    f.verify_rejected("/Parent")
}

#[test]
fn tree_missing_kid_reference_rejected() -> Result<()> {
    let mut f = Fixture::new()?;
    let missing = f.pdf.new_object_id();
    f.pdf.get_object_mut(f.root)?.as_dict_mut()?
        .set("Kids", vec![f.first.into(), missing.into()]);
    // Minimal falsifier: lopdf's lenient iterator gives one page even
    // though the original root /Count demands two descendants. Shiva
    // must never accept this silently shortened page list as Complete.
    assert_eq!(f.pdf.get_pages().len(), 1);
    f.verify_rejected("")
}

#[test]
fn tree_duplicate_page_id_rejected() -> Result<()> {
    let mut f = Fixture::new()?;
    f.pdf.get_object_mut(f.root)?.as_dict_mut()?
        .set("Kids", vec![f.first.into(), f.first.into()]);
    f.verify_rejected("duplicate")
}

#[test]
fn tree_direct_kid_dictionary_rejected() -> Result<()> {
    let mut f = Fixture::new()?;
    f.pdf.get_object_mut(f.root)?.as_dict_mut()?.set("Kids", vec![
        f.first.into(),
        Object::Dictionary(dictionary! {"Type" => "Page"}),
    ]);
    f.verify_rejected("/Kids")
}

#[test]
fn tree_wrong_kid_type_rejected() -> Result<()> {
    let mut f = Fixture::new()?;
    let alien = f.pdf.add_object(dictionary! {
        "Type" => "Font",
        "Parent" => f.root,
    });
    f.pdf.get_object_mut(f.root)?.as_dict_mut()?
        .set("Kids", vec![f.first.into(), alien.into()]);
    f.verify_rejected("/Type")
}

#[test]
fn tree_root_page_instead_of_pages_rejected() -> Result<()> {
    let mut f = Fixture::new()?;
    let root = f.pdf.get_object_mut(f.root)?.as_dict_mut()?;
    root.set("Type", "Page");
    f.verify_rejected("root")
}

#[test]
fn tree_cycle_via_kids_rejected() -> Result<()> {
    let mut f = Fixture::new()?;
    f.pdf.get_object_mut(f.root)?.as_dict_mut()?
        .set("Kids", vec![f.first.into(), f.root.into()]);
    f.verify_rejected("page-tree")
}

#[test]
fn tree_valid_indirect_kids_array_keeps_page_order() -> Result<()> {
    let mut f = Fixture::new()?;
    let kids_array = f.pdf.add_object(Object::Array(vec![
        f.first.into(), f.second.into(),
    ]));
    f.pdf.get_object_mut(f.root)?.as_dict_mut()?.set("Kids", kids_array);
    assert_eq!(validate_page_tree(&f.pdf)?, vec![f.first, f.second]);
    let output = format!("{:?}", Transformer::parse(&f.bytes()?)?);
    assert_eq!(output.matches("TreeFirstUnique").count(), 1);
    assert_eq!(output.matches("TreeSecondUnique").count(), 1);
    Ok(())
}

#[test]
fn tree_reused_indirect_kids_array_rejected() -> Result<()> {
    let mut f = Fixture::new()?;
    let shared = f.pdf.add_object(Object::Array(vec![f.first.into()]));
    // Distinct /Pages branches, both claiming the *same* indirect /Kids
    // object. This must be rejected even before the second leaf's /Parent
    // mismatch could conceal the shared-array problem.
    let first_branch = f.pdf.add_object(dictionary! {
        "Type" => "Pages", "Parent" => f.root,
        "Kids" => shared, "Count" => 1,
    });
    let second_branch = f.pdf.add_object(dictionary! {
        "Type" => "Pages", "Parent" => f.root,
        "Kids" => shared, "Count" => 1,
    });
    f.pdf.get_object_mut(f.first)?.as_dict_mut()?.set("Parent", first_branch);
    f.pdf.get_object_mut(f.root)?.as_dict_mut()?.set("Kids", vec![
        first_branch.into(), second_branch.into(),
    ]);
    f.verify_rejected("indirect array is cyclic or shared")
}

#[test]
fn tree_indirect_kids_self_reference_rejected() -> Result<()> {
    let mut f = Fixture::new()?;
    let indirect = f.pdf.new_object_id();
    f.pdf.objects.insert(indirect, Object::Reference(indirect));
    f.pdf.get_object_mut(f.root)?.as_dict_mut()?.set("Kids", indirect);
    // lopdf's object reader itself may identify the self-reference
    // before the Shiva validator runs. Either way the parse must fail closed.
    f.verify_rejected("cycle")
}

#[test]
fn tree_zero_pages_rejected_explicitly() -> Result<()> {
    let mut f = Fixture::new()?;
    let root = f.pdf.get_object_mut(f.root)?.as_dict_mut()?;
    root.set("Kids", Vec::<Object>::new());
    root.set("Count", 0);
    f.verify_rejected("/Count")
}

#[test]
fn tree_depth_budget_rejected() -> Result<()> {
    let mut f = Fixture::new()?;
    let mut parent = f.root;
    for _ in 0..66 {
        let node = f.pdf.add_object(dictionary! {
            "Type" => "Pages", "Parent" => parent,
            "Kids" => Vec::<Object>::new(), "Count" => 1,
        });
        f.pdf.get_object_mut(parent)?.as_dict_mut()?
            .set("Kids", vec![node.into()]);
        parent = node;
    }
    f.pdf.get_object_mut(parent)?.as_dict_mut()?
        .set("Kids", vec![f.first.into()]);
    f.pdf.get_object_mut(f.first)?.as_dict_mut()?.set("Parent", parent);
    f.verify_rejected("depth")
}

#[test]
fn tree_declared_page_limit_rejected() -> Result<()> {
    let mut f = Fixture::new()?;
    f.pdf.get_object_mut(f.root)?.as_dict_mut()?
        .set("Count", (MAX_PDF_PAGES + 1) as i64);
    f.verify_rejected("/Count")
}
