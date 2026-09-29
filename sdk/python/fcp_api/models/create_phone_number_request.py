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





T = TypeVar("T", bound="CreatePhoneNumberRequest")



@_attrs_define
class CreatePhoneNumberRequest:
    """ Create phone number request.

        Attributes:
            e164 (str):
            tenant_id (str | Unset): A TenantId: `tenant-<uuid>`, or the bare UUID
            e911_address (E911AddressResponse | None | Unset):
            e911_callback (None | str | Unset):
            cnam (None | str | Unset):
            carrier (None | str | Unset):
            tags (list[str] | None | Unset):
            location_id (None | str | Unset):
            site_id (None | str | Unset): The site whose phones answer this number; empty to answer it here.
     """

    e164: str
    tenant_id: str | Unset = UNSET
    e911_address: E911AddressResponse | None | Unset = UNSET
    e911_callback: None | str | Unset = UNSET
    cnam: None | str | Unset = UNSET
    carrier: None | str | Unset = UNSET
    tags: list[str] | None | Unset = UNSET
    location_id: None | str | Unset = UNSET
    site_id: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.e911_address_response import E911AddressResponse # noqa: PLC0415
        e164 = self.e164

        tenant_id = self.tenant_id

        e911_address: dict[str, Any] | None | Unset
        if isinstance(self.e911_address, Unset):
            e911_address = UNSET
        elif isinstance(self.e911_address, E911AddressResponse):
            e911_address = self.e911_address.to_dict()
        else:
            e911_address = self.e911_address

        e911_callback: None | str | Unset
        if isinstance(self.e911_callback, Unset):
            e911_callback = UNSET
        else:
            e911_callback = self.e911_callback

        cnam: None | str | Unset
        if isinstance(self.cnam, Unset):
            cnam = UNSET
        else:
            cnam = self.cnam

        carrier: None | str | Unset
        if isinstance(self.carrier, Unset):
            carrier = UNSET
        else:
            carrier = self.carrier

        tags: list[str] | None | Unset
        if isinstance(self.tags, Unset):
            tags = UNSET
        elif isinstance(self.tags, list):
            tags = self.tags


        else:
            tags = self.tags

        location_id: None | str | Unset
        if isinstance(self.location_id, Unset):
            location_id = UNSET
        else:
            location_id = self.location_id

        site_id: None | str | Unset
        if isinstance(self.site_id, Unset):
            site_id = UNSET
        else:
            site_id = self.site_id


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "e164": e164,
        })
        if tenant_id is not UNSET:
            field_dict["tenant_id"] = tenant_id
        if e911_address is not UNSET:
            field_dict["e911_address"] = e911_address
        if e911_callback is not UNSET:
            field_dict["e911_callback"] = e911_callback
        if cnam is not UNSET:
            field_dict["cnam"] = cnam
        if carrier is not UNSET:
            field_dict["carrier"] = carrier
        if tags is not UNSET:
            field_dict["tags"] = tags
        if location_id is not UNSET:
            field_dict["location_id"] = location_id
        if site_id is not UNSET:
            field_dict["site_id"] = site_id

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.e911_address_response import E911AddressResponse # noqa: PLC0415
        d = dict(src_dict)
        e164 = d.pop("e164")

        tenant_id = d.pop("tenant_id", UNSET)

        def _parse_e911_address(data: object) -> E911AddressResponse | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                e911_address_type_0 = E911AddressResponse.from_dict(data)



                return e911_address_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(E911AddressResponse | None | Unset, data)

        e911_address = _parse_e911_address(d.pop("e911_address", UNSET))


        def _parse_e911_callback(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        e911_callback = _parse_e911_callback(d.pop("e911_callback", UNSET))


        def _parse_cnam(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        cnam = _parse_cnam(d.pop("cnam", UNSET))


        def _parse_carrier(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        carrier = _parse_carrier(d.pop("carrier", UNSET))


        def _parse_tags(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                tags_type_0 = cast(list[str], data)

                return tags_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        tags = _parse_tags(d.pop("tags", UNSET))


        def _parse_location_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        location_id = _parse_location_id(d.pop("location_id", UNSET))


        def _parse_site_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        site_id = _parse_site_id(d.pop("site_id", UNSET))


        create_phone_number_request = cls(
            e164=e164,
            tenant_id=tenant_id,
            e911_address=e911_address,
            e911_callback=e911_callback,
            cnam=cnam,
            carrier=carrier,
            tags=tags,
            location_id=location_id,
            site_id=site_id,
        )


        create_phone_number_request.additional_properties = d
        return create_phone_number_request

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
