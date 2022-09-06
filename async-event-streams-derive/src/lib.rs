//!
//! Crate allows to automatically derive [EventSink](async_event_streams::EventSink) trait for any
//! structure implementing [EventSintExt](async_event_streams::EventSinkExt)
//!
//! Usage sample:
//! ```
//! use std::sync::Arc;
//! use async_trait::async_trait;
//! use std::borrow::Cow;
//! use async_event_streams::{EventBox, EventSink, EventSinkExt};
//! use async_event_streams_derive::EventSink;
//!
//! #[derive(EventSink)]
//! #[event_sink(event=Event)]
//! struct Sink;
//!
//! #[derive(Clone)]
//! struct Event;
//!
//! #[async_trait]
//! impl EventSinkExt<Event> for Sink {
//!     type Error = ();
//!     async fn on_event<'a>(&'a self, event: Cow<'a, Event>, source: Option<Arc<EventBox>>) -> Result<(), Self::Error> {
//!         todo!()
//!     }
//!}
//! ```
mod param;

use param::{take_params, Param};
use quote::quote;
use syn::{parse::ParseStream, parse_macro_input, DeriveInput, Ident};

extern crate proc_macro;

#[proc_macro_derive(EventSink, attributes(event_sink))]
pub fn derive_event_sink(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input: DeriveInput = parse_macro_input!(input);
    match derive_event_sink_impl(input) {
        Ok(stream) => stream,
        Err(e) => e.into_compile_error(),
    }
    .into()
}

fn derive_event_sink_impl(mut input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let mut derives = Vec::new();
    if let Some(params) = take_params::<StructParam>("event_sink", &mut input.attrs)? {
        for param in params {
            match param {
                StructParam::Event(Ok(v)) => {
                    derives.push(derive_event_sink_for_event(&input.ident, v)?)
                }
                StructParam::Event(Err(e)) => return Err(e),
            }
        }
    }
    let mut res = proc_macro2::TokenStream::new();
    res.extend(derives.into_iter());
    Ok(res)
}

fn derive_event_sink_for_event(
    struct_id: &Ident,
    event_id: Ident,
) -> syn::Result<proc_macro2::TokenStream> {
    Ok(quote! {
        #[async_trait]
        impl EventSink<#event_id> for #struct_id {
            type Error = <#struct_id as EventSinkExt<#event_id>>::Error;
            async fn on_event_ref(
                &self,
                event: &#event_id,
                source: Option<Arc<EventBox>>,
            ) -> Result<(), Self::Error> {
                <#struct_id as EventSinkExt<#event_id>>::on_event(&self, Cow::Borrowed(event), source).await
            }
            async fn on_event_owned(
                &self,
                event: #event_id,
                source: Option<Arc<EventBox>>,
            ) -> Result<(), Self::Error> {
                <#struct_id as EventSinkExt<#event_id>>::on_event(&self, Cow::Owned(event), source).await
            }
        }
    })
}

enum StructParam {
    Event(Result<Ident, syn::Error>),
}

impl Param for StructParam {
    fn default(name: Ident) -> syn::Result<Self> {
        if name == "event" {
            return Ok(StructParam::Event(Err(syn::Error::new(
                name.span(),
                "Event name expected",
            ))));
        } else {
            Err(syn::Error::new(
                name.span(),
                "Unexpected parameter. Allowed value is 'event'",
            ))
        }
    }
    fn parse(&mut self, input: ParseStream) -> syn::Result<()> {
        match self {
            StructParam::Event(ref mut v) => *v = Ok(input.parse()?),
        }
        Ok(())
    }
}
