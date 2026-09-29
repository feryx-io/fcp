from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="PageLibraryTemplateResponseDataItem")



@_attrs_define
class PageLibraryTemplateResponseDataItem:
    """ Built-in template library entry response.

        Attributes:
            vendor (str):
            name (str):
            description (str):
            template_body (str):
            model_pattern (None | str | Unset):
     """

    vendor: str
    name: str
    description: str
    template_body: str
    model_pattern: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        vendor = self.vendor

        name = self.name

        description = self.description

        template_body = self.template_body

        model_pattern: None | str | Unset
        if isinstance(self.model_pattern, Unset):
            model_pattern = UNSET
        else:
            model_pattern = self.model_pattern


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "vendor": vendor,
            "name": name,
            "description": description,
            "template_body": template_body,
        })
        if model_pattern is not UNSET:
            field_dict["model_pattern"] = model_pattern

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        vendor = d.pop("vendor")

        name = d.pop("name")

        description = d.pop("description")

        template_body = d.pop("template_body")

        def _parse_model_pattern(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        model_pattern = _parse_model_pattern(d.pop("model_pattern", UNSET))


        page_library_template_response_data_item = cls(
            vendor=vendor,
            name=name,
            description=description,
            template_body=template_body,
            model_pattern=model_pattern,
        )


        page_library_template_response_data_item.additional_properties = d
        return page_library_template_response_data_item

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
