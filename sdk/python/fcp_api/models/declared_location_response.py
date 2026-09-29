from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.e911_address_response import E911AddressResponse





T = TypeVar("T", bound="DeclaredLocationResponse")



@_attrs_define
class DeclaredLocationResponse:
    """ A user's declared location: where they are when no network places them.

        Attributes:
            address (E911AddressResponse):
            declared_at (datetime.datetime):
            expires_at (datetime.datetime):
            current (bool): False once it has expired: the platform stops using it.
     """

    address: E911AddressResponse
    declared_at: datetime.datetime
    expires_at: datetime.datetime
    current: bool
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.e911_address_response import E911AddressResponse # noqa: PLC0415
        address = self.address.to_dict()

        declared_at = self.declared_at.isoformat()

        expires_at = self.expires_at.isoformat()

        current = self.current


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "address": address,
            "declared_at": declared_at,
            "expires_at": expires_at,
            "current": current,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.e911_address_response import E911AddressResponse # noqa: PLC0415
        d = dict(src_dict)
        address = E911AddressResponse.from_dict(d.pop("address"))




        declared_at = datetime.datetime.fromisoformat(d.pop("declared_at"))




        expires_at = datetime.datetime.fromisoformat(d.pop("expires_at"))




        current = d.pop("current")

        declared_location_response = cls(
            address=address,
            declared_at=declared_at,
            expires_at=expires_at,
            current=current,
        )


        declared_location_response.additional_properties = d
        return declared_location_response

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
