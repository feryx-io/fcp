from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.device_response import DeviceResponse





T = TypeVar("T", bound="ImportDevicesResponse")



@_attrs_define
class ImportDevicesResponse:
    """ What an import made: each device with its provisioning token, shown
    this once.

        Attributes:
            created (list[DeviceResponse]):
     """

    created: list[DeviceResponse]
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.device_response import DeviceResponse # noqa: PLC0415
        created = []
        for created_item_data in self.created:
            created_item = created_item_data.to_dict()
            created.append(created_item)




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "created": created,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.device_response import DeviceResponse # noqa: PLC0415
        d = dict(src_dict)
        created = []
        _created = d.pop("created")
        for created_item_data in (_created):
            created_item = DeviceResponse.from_dict(created_item_data)



            created.append(created_item)


        import_devices_response = cls(
            created=created,
        )


        import_devices_response.additional_properties = d
        return import_devices_response

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
