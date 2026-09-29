from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="UpdateConfigTemplateRequest")



@_attrs_define
class UpdateConfigTemplateRequest:
    """ Update config template request.

        Attributes:
            name (None | str | Unset):
            vendor (None | str | Unset):
            model_pattern (None | str | Unset):
            template_body (None | str | Unset):
            description (None | str | Unset):
     """

    name: None | str | Unset = UNSET
    vendor: None | str | Unset = UNSET
    model_pattern: None | str | Unset = UNSET
    template_body: None | str | Unset = UNSET
    description: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name

        vendor: None | str | Unset
        if isinstance(self.vendor, Unset):
            vendor = UNSET
        else:
            vendor = self.vendor

        model_pattern: None | str | Unset
        if isinstance(self.model_pattern, Unset):
            model_pattern = UNSET
        else:
            model_pattern = self.model_pattern

        template_body: None | str | Unset
        if isinstance(self.template_body, Unset):
            template_body = UNSET
        else:
            template_body = self.template_body

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if name is not UNSET:
            field_dict["name"] = name
        if vendor is not UNSET:
            field_dict["vendor"] = vendor
        if model_pattern is not UNSET:
            field_dict["model_pattern"] = model_pattern
        if template_body is not UNSET:
            field_dict["template_body"] = template_body
        if description is not UNSET:
            field_dict["description"] = description

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


        def _parse_vendor(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        vendor = _parse_vendor(d.pop("vendor", UNSET))


        def _parse_model_pattern(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        model_pattern = _parse_model_pattern(d.pop("model_pattern", UNSET))


        def _parse_template_body(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        template_body = _parse_template_body(d.pop("template_body", UNSET))


        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        update_config_template_request = cls(
            name=name,
            vendor=vendor,
            model_pattern=model_pattern,
            template_body=template_body,
            description=description,
        )


        update_config_template_request.additional_properties = d
        return update_config_template_request

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
