from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="RenderedConfigResponse")



@_attrs_define
class RenderedConfigResponse:
    """ A rendered configuration.

        Attributes:
            vendor (str):
            content (str):
            config_template_id (None | str | Unset): The template it came from, when it is a saved one.
            config_template_name (None | str | Unset):
     """

    vendor: str
    content: str
    config_template_id: None | str | Unset = UNSET
    config_template_name: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        vendor = self.vendor

        content = self.content

        config_template_id: None | str | Unset
        if isinstance(self.config_template_id, Unset):
            config_template_id = UNSET
        else:
            config_template_id = self.config_template_id

        config_template_name: None | str | Unset
        if isinstance(self.config_template_name, Unset):
            config_template_name = UNSET
        else:
            config_template_name = self.config_template_name


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "vendor": vendor,
            "content": content,
        })
        if config_template_id is not UNSET:
            field_dict["config_template_id"] = config_template_id
        if config_template_name is not UNSET:
            field_dict["config_template_name"] = config_template_name

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        vendor = d.pop("vendor")

        content = d.pop("content")

        def _parse_config_template_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        config_template_id = _parse_config_template_id(d.pop("config_template_id", UNSET))


        def _parse_config_template_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        config_template_name = _parse_config_template_name(d.pop("config_template_name", UNSET))


        rendered_config_response = cls(
            vendor=vendor,
            content=content,
            config_template_id=config_template_id,
            config_template_name=config_template_name,
        )


        rendered_config_response.additional_properties = d
        return rendered_config_response

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
