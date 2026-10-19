//! Read compiled manifests independently of the XML generator. Reject permissions
//! nested under application, missing Android namespaces, duplicates or wrong types.
use std::collections::BTreeSet;
const ANDROID: &str = "http://schemas.android.com/apk/res/android";
fn u16_at(b: &[u8], p: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(b.get(p..p+2).ok_or("truncated AXML")?.try_into().unwrap()))
}
fn u32_at(b: &[u8], p: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(b.get(p..p+4).ok_or("truncated AXML")?.try_into().unwrap()))
}
fn string(strings: &[String], index: u32) -> Result<&str, String> {
    strings.get(index as usize).map(String::as_str).ok_or_else(|| "invalid AXML string reference".into())
}
fn length(b: &[u8], p: &mut usize) -> Result<usize, String> {
    let first = *b.get(*p).ok_or("truncated string length")?; *p += 1;
    if first & 0x80 == 0 { return Ok(first as usize); }
    let second = *b.get(*p).ok_or("truncated string length")?; *p += 1;
    Ok((((first & 0x7f) as usize) << 8) | second as usize)
}
pub fn apk_permissions(bytes: &[u8]) -> Result<BTreeSet<String>, String> {
    if u16_at(bytes,0)? != 3 || u32_at(bytes,4)? as usize != bytes.len() { return Err("invalid AXML root".into()); }
    let mut position = u16_at(bytes,2)? as usize;
    let mut strings = Vec::new();
    let mut resources = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut permissions = BTreeSet::new();
    let mut root = false;
    while position < bytes.len() {
        let size = u32_at(bytes,position+4)? as usize;
        if size < 8 { return Err("invalid AXML chunk size".into()); }
        let chunk = bytes.get(position..position.checked_add(size).ok_or("AXML size overflow")?).ok_or("truncated AXML chunk")?;
        match u16_at(chunk,0)? {
            1 => {
                if u32_at(chunk,16)? & 0x100 == 0 { return Err("expected compiler's UTF8 string pool".into()); }
                let count = u32_at(chunk,8)? as usize;
                let start = u32_at(chunk,20)? as usize;
                let header = u16_at(chunk,2)? as usize;
                if count > size/4 { return Err("invalid string count".into()); }
                for i in 0..count {
                    let mut p = start.checked_add(u32_at(chunk,header+i*4)? as usize).ok_or("string overflow")?;
                    length(chunk,&mut p)?;
                    let len = length(chunk,&mut p)?;
                    let text = chunk.get(p..p+len).ok_or("truncated string")?;
                    strings.push(std::str::from_utf8(text).map_err(|e|e.to_string())?.to_string());
                }
            }
            0x180 => {
                for p in (8..size).step_by(4) { resources.push(u32_at(chunk,p)?); }
            }
            0x102 => {
                let name = string(&strings,u32_at(chunk,20)?)?;
                if stack.is_empty() {
                    if root || name != "manifest" { return Err("manifest root missing or duplicated".into()); }
                    root = true;
                }
                if name == "uses-permission" {
                    if stack.as_slice() != ["manifest"] { return Err("uses-permission is not a direct manifest child".into()); }
                    let start = 16 + u16_at(chunk,24)? as usize;
                    let stride = u16_at(chunk,26)? as usize;
                    let count = u16_at(chunk,28)? as usize;
                    if stride < 20 { return Err("invalid AXML attribute size".into()); }
                    let mut permission = None;
                    for i in 0..count {
                        let p = start+i*stride;
                        let name_id = u32_at(chunk,p+4)?;
                        let attr = string(&strings,name_id)?;
                        if attr == "name" {
                            if string(&strings,u32_at(chunk,p)?)? != ANDROID || resources.get(name_id as usize) != Some(&0x01010003) {
                                return Err("permission name is not android:name".into());
                            }
                            if chunk.get(p+15) != Some(&3) { return Err("permission name is not a typed string".into()); }
                            permission = Some(string(&strings,u32_at(chunk,p+16)?)?.to_string());
                        }
                    }
                    let permission = permission.ok_or("permission name missing")?;
                    if !permissions.insert(permission) { return Err("duplicate uses-permission".into()); }
                }
                stack.push(name.to_string());
            }
            0x103 => {
                let name = string(&strings,u32_at(chunk,20)?)?;
                if stack.pop().as_deref() != Some(name) { return Err("unbalanced AXML elements".into()); }
            }
            _ => {}
        }
        position += size;
    }
    if !root || !stack.is_empty() { return Err("incomplete manifest".into()); }
    Ok(permissions)
}

#[cfg(feature="aab")]
pub fn aab_permissions(bytes: &[u8]) -> Result<BTreeSet<String>, String> {
    use prost::Message;
    use pack_aab::aapt::pb::{XmlNode, xml_node::Node};
    fn visit(node: &XmlNode, parent: &str, depth: usize, permissions: &mut BTreeSet<String>) -> Result<(),String> {
        if depth > 100 { return Err("manifest nesting exceeds limit".into()); }
        if let Some(Node::Element(element)) = &node.node {
            if depth == 0 && element.name != "manifest" { return Err("manifest root missing".into()); }
            if element.name == "uses-permission" {
                if parent != "manifest" || depth != 1 { return Err("uses-permission is not a direct manifest child".into()); }
                let attribute = element.attribute.iter().find(|a|a.name=="name" && a.namespace_uri==ANDROID).ok_or("permission android:name missing")?;
                if attribute.resource_id != 0x01010003 || attribute.value.is_empty() { return Err("invalid permission attribute".into()); }
                if !permissions.insert(attribute.value.clone()) { return Err("duplicate uses-permission".into()); }
            }
            for child in &element.child { visit(child,&element.name,depth+1,permissions)?; }
        } else if depth == 0 { return Err("manifest root missing".into()); }
        Ok(())
    }
    let node = XmlNode::decode(bytes).map_err(|e| e.to_string())?;
    let mut permissions = BTreeSet::new(); visit(&node,"",0,&mut permissions)?; Ok(permissions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use deku::DekuContainerWrite;
    #[test]
    fn permissions_under_application_are_rejected() {
        let xml = br#"<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="dev.test"><application><uses-permission android:name="android.permission.CAMERA" /></application></manifest>"#;
        let (chunk,_) = pack_asset_compiler::xml_file::xml_to_res_chunk(&mut std::io::Cursor::new(xml),&[]).unwrap();
        let error=apk_permissions(&chunk.to_bytes().unwrap()).unwrap_err();
        assert!(error.contains("direct manifest child"));
    }
    #[test]
    fn malformed_binary_is_an_error() {
        for length in 0..32 {
            assert!(apk_permissions(&vec![0;length]).is_err());
        }
    }
}
