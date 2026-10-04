from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.preview_template_response_context import PreviewTemplateResponseContext





T = TypeVar("T", bound="PreviewTemplateResponse")



@_attrs_define
class PreviewTemplateResponse:
    """ A body rendered against sample values.

        Attributes:
            content (str):
            context (PreviewTemplateResponseContext): The values it was rendered with (`docs/PROVISIONING.md` §5).
     """

    content: str
    context: PreviewTemplateResponseContext
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.preview_template_response_context import PreviewTemplateResponseContext # noqa: PLC0415
        content = self.content

        context = self.context.to_dict()


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "content": content,
            "context": context,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.preview_template_response_context import PreviewTemplateResponseContext # noqa: PLC0415
        d = dict(src_dict)
        content = d.pop("content")

        context = PreviewTemplateResponseContext.from_dict(d.pop("context"))




        preview_template_response = cls(
            content=content,
            context=context,
        )


        preview_template_response.additional_properties = d
        return preview_template_response

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
