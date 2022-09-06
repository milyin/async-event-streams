use proc_macro2::Ident;
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    Attribute, Token,
};

pub trait Param: Sized {
    fn default(name: Ident) -> syn::Result<Self>;
    fn parse(&mut self, input: ParseStream) -> syn::Result<()>;
}

struct ParamParser<P: Param>(P);

impl<P: Param> Parse for ParamParser<P> {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        let mut param = P::default(ident)?;
        if let Ok(_) = input.parse::<Token![=]>() {
            param.parse(input)?;
        }
        Ok(Self(param))
    }
}

fn is_attr_name_match(attr: &Attribute, name: &str) -> bool {
    if let Some(attr_ident) = attr.path.get_ident() {
        if attr_ident == name {
            return true;
        }
    }
    false
}

pub fn take_params<T: Param>(
    attr_name: &str,
    attrs: &mut Vec<Attribute>,
) -> syn::Result<Option<Vec<T>>> {
    let mut taken_attrs = Vec::new();
    attrs.retain(|attr| {
        if is_attr_name_match(attr, attr_name) {
            taken_attrs.push(attr.clone());
            false
        } else {
            true
        }
    });
    if taken_attrs.is_empty() {
        return Ok(None);
    }

    let mut res = Vec::new();
    for attr in taken_attrs {
        if !attr.tokens.is_empty() {
            let params = attr.parse_args_with(
                Punctuated::<ParamParser<T>, Token![,]>::parse_separated_nonempty,
            )?;
            res.extend(params.into_iter().map(|ParamParser(param)| param));
        }
    }
    Ok(Some(res))
}
