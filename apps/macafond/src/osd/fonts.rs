//! The same local OFL fonts as the UI, embedded so the native popup works with the UI closed.
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::{Interface, PCWSTR, Result, w};

struct RegisteredLoader {
    factory: IDWriteFactory,
    loader: IDWriteInMemoryFontFileLoader,
}
impl Drop for RegisteredLoader {
    fn drop(&mut self) {
        // SAFETY: registration belongs to this object; font collections/formats drop first.
        unsafe {
            let _ = self.factory.UnregisterFontFileLoader(&self.loader);
        }
    }
}

pub struct Fonts {
    collection: IDWriteFontCollection,
    registered: RegisteredLoader,
}
impl Fonts {
    pub fn new(factory: &IDWriteFactory) -> Result<Self> {
        // SAFETY: immutable embedded bytes; DirectWrite copies each expanded font when the
        // owner object is null. Fragment pointers are released even if copying fails.
        unsafe {
            let modern: IDWriteFactory5 = factory.cast()?;
            let loader = modern.CreateInMemoryFontFileLoader()?;
            factory.RegisterFontFileLoader(&loader)?;
            let registered = RegisteredLoader {
                factory: factory.clone(),
                loader,
            };
            let builder = modern.CreateFontSetBuilder()?;
            for data in [
                include_bytes!("../../assets/fonts/geist-mono.woff2").as_slice(),
                include_bytes!("../../assets/fonts/inter.woff2").as_slice(),
                include_bytes!("../../assets/fonts/instrument-serif-italic.woff2").as_slice(),
            ] {
                let stream = modern.UnpackFontFile(
                    DWRITE_CONTAINER_TYPE_WOFF2,
                    data.as_ptr().cast(),
                    data.len() as u32,
                )?;
                let size = stream.GetFileSize()?;
                let size32 = u32::try_from(size).map_err(|_| {
                    windows::core::Error::from_hresult(windows::Win32::Foundation::E_INVALIDARG)
                })?;
                let mut bytes = std::ptr::null_mut();
                let mut context = std::ptr::null_mut();
                stream.ReadFileFragment(&mut bytes, 0, size, &mut context)?;
                let font = registered
                    .loader
                    .CreateInMemoryFontFileReference(factory, bytes, size32, None);
                stream.ReleaseFileFragment(context);
                builder.AddFontFile(&font?)?;
            }
            let set = builder.CreateFontSet()?;
            let older: IDWriteFactory3 = factory.cast()?;
            let collection = older.CreateFontCollectionFromFontSet(&set)?.cast()?;
            Ok(Self {
                collection,
                registered,
            })
        }
    }
    pub fn format(
        &self,
        family: PCWSTR,
        weight: DWRITE_FONT_WEIGHT,
        size: f32,
        centered: bool,
        italic: bool,
    ) -> Result<IDWriteTextFormat> {
        // SAFETY: the collection is retained until every popup format is dropped.
        unsafe {
            let format = self.registered.factory.CreateTextFormat(
                family,
                &self.collection,
                weight,
                if italic {
                    DWRITE_FONT_STYLE_ITALIC
                } else {
                    DWRITE_FONT_STYLE_NORMAL
                },
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("en-us"),
            )?;
            format.SetTextAlignment(if centered {
                DWRITE_TEXT_ALIGNMENT_CENTER
            } else {
                DWRITE_TEXT_ALIGNMENT_LEADING
            })?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            Ok(format)
        }
    }
}
