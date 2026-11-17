//! Ferramentas de introspecção semântica, locators e diagnósticos de layout para agentes.

use crate::{Bounds, Pixels, VisualTestContext};
use serde::{Deserialize, Serialize};

/// Informações semânticas e espaciais de um elemento na árvore do GPUI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ElementSnapshot {
    /// Nome do seletor ou chave de debug registrada
    pub selector: String,
    /// Coordenada X computada pelo layout
    pub x: f32,
    /// Coordenada Y computada pelo layout
    pub y: f32,
    /// Largura computada do elemento
    pub width: f32,
    /// Altura computada do elemento
    pub height: f32,
    /// Se a área é não-nula e visível na viewport
    pub is_visible: bool,
}

/// Árvore de elementos visualizada no frame atual para inspeção do agente.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiTreeSnapshot {
    /// Quantidade total de elementos registrados no frame
    pub total_elements: usize,
    /// Lista dos snapshots de cada elemento do frame
    pub elements: Vec<ElementSnapshot>,
}

impl UiTreeSnapshot {
    /// Localiza o primeiro elemento cujo seletor contenha ou coincida com a chave informada
    pub fn find(&self, query: &str) -> Option<&ElementSnapshot> {
        self.elements
            .iter()
            .find(|e| e.selector == query || e.selector.contains(query))
    }

    /// Retorna todos os elementos que combinam com o seletor informado
    pub fn find_all(&self, query: &str) -> Vec<&ElementSnapshot> {
        self.elements
            .iter()
            .filter(|e| e.selector == query || e.selector.contains(query))
            .collect()
    }
}

/// Helper para interagir com elementos encontrados semânticamente (estilo Playwright Locator)
pub struct ElementLocator<'a> {
    cx: &'a mut VisualTestContext,
    selector: String,
    bounds: Option<Bounds<Pixels>>,
}

impl<'a> ElementLocator<'a> {
    /// Cria um novo `ElementLocator` para o seletor informado
    pub fn new(cx: &'a mut VisualTestContext, selector: impl Into<String>) -> Self {
        let sel = selector.into();
        let bounds = cx.update(|window, _| window.rendered_frame.debug_bounds.get(&sel).copied());
        Self {
            cx,
            selector: sel,
            bounds,
        }
    }

    /// Verifica se o elemento está presente na árvore e possui dimensões maiores que 0x0
    pub fn is_visible(&self) -> bool {
        self.bounds.map_or(false, |b| {
            b.size.width > Pixels::ZERO && b.size.height > Pixels::ZERO
        })
    }

    /// Retorna os bounds do elemento
    pub fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.bounds
    }

    /// Simula clique primário no centro do elemento
    pub fn click(&mut self) -> Result<(), String> {
        let Some(bounds) = self.bounds else {
            return Err(format!(
                "Elemento com seletor '{}' não encontrado no frame",
                self.selector
            ));
        };
        if bounds.size.width <= Pixels::ZERO || bounds.size.height <= Pixels::ZERO {
            return Err(format!(
                "Elemento '{}' possui dimensões zeradas ({:?})",
                self.selector, bounds.size
            ));
        }

        self.cx.simulate_click(bounds.center(), Default::default());
        self.cx.run_until_parked();
        Ok(())
    }

    /// Simula digitação com o foco no elemento
    pub fn type_text(&mut self, text: &str) -> Result<(), String> {
        self.click()?;
        self.cx.simulate_input(text);
        self.cx.run_until_parked();
        Ok(())
    }
}

impl VisualTestContext {
    /// Captura um snapshot completo da árvore de elementos registrados no frame atual
    pub fn dump_ui_tree(&mut self) -> UiTreeSnapshot {
        self.update(|window, _| {
            let mut elements = Vec::new();
            for (selector, bounds) in &window.rendered_frame.debug_bounds {
                let x: f32 = bounds.origin.x.into();
                let y: f32 = bounds.origin.y.into();
                let width: f32 = bounds.size.width.into();
                let height: f32 = bounds.size.height.into();
                let is_visible = width > 0.0 && height > 0.0;

                elements.push(ElementSnapshot {
                    selector: selector.clone(),
                    x,
                    y,
                    width,
                    height,
                    is_visible,
                });
            }

            elements.sort_by(|a, b| a.selector.cmp(&b.selector));
            UiTreeSnapshot {
                total_elements: elements.len(),
                elements,
            }
        })
    }

    /// Cria um Locator semântico para encontrar e manipular um elemento
    pub fn locator<'a>(&'a mut self, selector: impl Into<String>) -> ElementLocator<'a> {
        ElementLocator::new(self, selector)
    }

    /// Diagnóstico automático de inconsistências de layout no frame
    pub fn lint_layout(&mut self) -> Vec<String> {
        let tree = self.dump_ui_tree();
        let mut warnings = Vec::new();

        for el in &tree.elements {
            if el.width <= 0.0 || el.height <= 0.0 {
                warnings.push(format!(
                    "[GPUI-LINT] ZeroSize: Elemento '{}' colapsou para tamanho ({:.1} x {:.1})",
                    el.selector, el.width, el.height
                ));
            }
        }

        warnings
    }
}
