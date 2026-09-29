from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.e911_address_response import E911AddressResponse





T = TypeVar("T", bound="EmergencyLocationResponse")



@_attrs_define
class EmergencyLocationResponse:
    """ 
        Attributes:
            address (E911AddressResponse):
            source (str):
            summary (str):
            callback_number (None | str | Unset):
            location_id (None | str | Unset):
            location_name (None | str | Unset):
            client_address (None | str | Unset): The address the caller was seen at, when one placed them.
            declared_age_secs (int | None | Unset): For a declared location, how long ago the user declared it.
     """

    address: E911AddressResponse
    source: str
    summary: str
    callback_number: None | str | Unset = UNSET
    location_id: None | str | Unset = UNSET
    location_name: None | str | Unset = UNSET
    client_address: None | str | Unset = UNSET
    declared_age_secs: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.e911_address_response import E911AddressResponse # noqa: PLC0415
        address = self.address.to_dict()

        source = self.source

        summary = self.summary

        callback_number: None | str | Unset
        if isinstance(self.callback_number, Unset):
            callback_number = UNSET
        else:
            callback_number = self.callback_number

        location_id: None | str | Unset
        if isinstance(self.location_id, Unset):
            location_id = UNSET
        else:
            location_id = self.location_id

        location_name: None | str | Unset
        if isinstance(self.location_name, Unset):
            location_name = UNSET
        else:
            location_name = self.location_name

        client_address: None | str | Unset
        if isinstance(self.client_address, Unset):
            client_address = UNSET
        else:
            client_address = self.client_address

        declared_age_secs: int | None | Unset
        if isinstance(self.declared_age_secs, Unset):
            declared_age_secs = UNSET
        else:
            declared_age_secs = self.declared_age_secs


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "address": address,
            "source": source,
            "summary": summary,
        })
        if callback_number is not UNSET:
            field_dict["callback_number"] = callback_number
        if location_id is not UNSET:
            field_dict["location_id"] = location_id
        if location_name is not UNSET:
            field_dict["location_name"] = location_name
        if client_address is not UNSET:
            field_dict["client_address"] = client_address
        if declared_age_secs is not UNSET:
            field_dict["declared_age_secs"] = declared_age_secs

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.e911_address_response import E911AddressResponse # noqa: PLC0415
        d = dict(src_dict)
        address = E911AddressResponse.from_dict(d.pop("address"))




        source = d.pop("source")

        summary = d.pop("summary")

        def _parse_callback_number(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        callback_number = _parse_callback_number(d.pop("callback_number", UNSET))


        def _parse_location_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        location_id = _parse_location_id(d.pop("location_id", UNSET))


        def _parse_location_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        location_name = _parse_location_name(d.pop("location_name", UNSET))


        def _parse_client_address(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        client_address = _parse_client_address(d.pop("client_address", UNSET))


        def _parse_declared_age_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        declared_age_secs = _parse_declared_age_secs(d.pop("declared_age_secs", UNSET))


        emergency_location_response = cls(
            address=address,
            source=source,
            summary=summary,
            callback_number=callback_number,
            location_id=location_id,
            location_name=location_name,
            client_address=client_address,
            declared_age_secs=declared_age_secs,
        )


        emergency_location_response.additional_properties = d
        return emergency_location_response

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
