from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="RenderDeviceRequest")



@_attrs_define
class RenderDeviceRequest:
    """ A template to try, in place of the device's.

        Attributes:
            config_template_id (None | str | Unset): One of the tenant's templates.
            template_body (None | str | Unset): A body not yet saved, rendered as `vendor`'s.
            vendor (None | str | Unset):
     """

    config_template_id: None | str | Unset = UNSET
    template_body: None | str | Unset = UNSET
    vendor: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        config_template_id: None | str | Unset
        if isinstance(self.config_template_id, Unset):
            config_template_id = UNSET
        else:
            config_template_id = self.config_template_id

        template_body: None | str | Unset
        if isinstance(self.template_body, Unset):
            template_body = UNSET
        else:
            template_body = self.template_body

        vendor: None | str | Unset
        if isinstance(self.vendor, Unset):
            vendor = UNSET
        else:
            vendor = self.vendor


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if config_template_id is not UNSET:
            field_dict["config_template_id"] = config_template_id
        if template_body is not UNSET:
            field_dict["template_body"] = template_body
        if vendor is not UNSET:
            field_dict["vendor"] = vendor

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_config_template_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        config_template_id = _parse_config_template_id(d.pop("config_template_id", UNSET))


        def _parse_template_body(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        template_body = _parse_template_body(d.pop("template_body", UNSET))


        def _parse_vendor(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        vendor = _parse_vendor(d.pop("vendor", UNSET))


        render_device_request = cls(
            config_template_id=config_template_id,
            template_body=template_body,
            vendor=vendor,
        )


        render_device_request.additional_properties = d
        return render_device_request

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
