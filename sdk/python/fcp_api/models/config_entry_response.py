from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.config_entry_response_value import ConfigEntryResponseValue





T = TypeVar("T", bound="ConfigEntryResponse")



@_attrs_define
class ConfigEntryResponse:
    """ 
        Attributes:
            service (str):
            section (str):
            tenant_id (str):
            value (ConfigEntryResponseValue): A configuration section: a JSON object.
            version (int):
            updated_at (str):
            updated_by (None | str | Unset):
     """

    service: str
    section: str
    tenant_id: str
    value: ConfigEntryResponseValue
    version: int
    updated_at: str
    updated_by: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.config_entry_response_value import ConfigEntryResponseValue # noqa: PLC0415
        service = self.service

        section = self.section

        tenant_id = self.tenant_id

        value = self.value.to_dict()

        version = self.version

        updated_at = self.updated_at

        updated_by: None | str | Unset
        if isinstance(self.updated_by, Unset):
            updated_by = UNSET
        else:
            updated_by = self.updated_by


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "service": service,
            "section": section,
            "tenant_id": tenant_id,
            "value": value,
            "version": version,
            "updated_at": updated_at,
        })
        if updated_by is not UNSET:
            field_dict["updated_by"] = updated_by

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.config_entry_response_value import ConfigEntryResponseValue # noqa: PLC0415
        d = dict(src_dict)
        service = d.pop("service")

        section = d.pop("section")

        tenant_id = d.pop("tenant_id")

        value = ConfigEntryResponseValue.from_dict(d.pop("value"))




        version = d.pop("version")

        updated_at = d.pop("updated_at")

        def _parse_updated_by(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        updated_by = _parse_updated_by(d.pop("updated_by", UNSET))


        config_entry_response = cls(
            service=service,
            section=section,
            tenant_id=tenant_id,
            value=value,
            version=version,
            updated_at=updated_at,
            updated_by=updated_by,
        )


        config_entry_response.additional_properties = d
        return config_entry_response

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
