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





T = TypeVar("T", bound="UpdatePhoneNumberRequest")



@_attrs_define
class UpdatePhoneNumberRequest:
    """ Update phone number request.

        Attributes:
            state (None | str | Unset):
            e911_address (E911AddressResponse | None | Unset):
            e911_callback (None | str | Unset):
            cnam (None | str | Unset):
            recording_enabled (bool | None | Unset):
            block_anonymous (bool | None | Unset):
            allow_list (list[str] | None | Unset):
            block_list (list[str] | None | Unset):
            tags (list[str] | None | Unset):
            location_id (None | str | Unset):
            site_id (None | str | Unset): The site whose phones answer this number; empty to answer it here.
     """

    state: None | str | Unset = UNSET
    e911_address: E911AddressResponse | None | Unset = UNSET
    e911_callback: None | str | Unset = UNSET
    cnam: None | str | Unset = UNSET
    recording_enabled: bool | None | Unset = UNSET
    block_anonymous: bool | None | Unset = UNSET
    allow_list: list[str] | None | Unset = UNSET
    block_list: list[str] | None | Unset = UNSET
    tags: list[str] | None | Unset = UNSET
    location_id: None | str | Unset = UNSET
    site_id: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.e911_address_response import E911AddressResponse # noqa: PLC0415
        state: None | str | Unset
        if isinstance(self.state, Unset):
            state = UNSET
        else:
            state = self.state

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

        recording_enabled: bool | None | Unset
        if isinstance(self.recording_enabled, Unset):
            recording_enabled = UNSET
        else:
            recording_enabled = self.recording_enabled

        block_anonymous: bool | None | Unset
        if isinstance(self.block_anonymous, Unset):
            block_anonymous = UNSET
        else:
            block_anonymous = self.block_anonymous

        allow_list: list[str] | None | Unset
        if isinstance(self.allow_list, Unset):
            allow_list = UNSET
        elif isinstance(self.allow_list, list):
            allow_list = self.allow_list


        else:
            allow_list = self.allow_list

        block_list: list[str] | None | Unset
        if isinstance(self.block_list, Unset):
            block_list = UNSET
        elif isinstance(self.block_list, list):
            block_list = self.block_list


        else:
            block_list = self.block_list

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
        })
        if state is not UNSET:
            field_dict["state"] = state
        if e911_address is not UNSET:
            field_dict["e911_address"] = e911_address
        if e911_callback is not UNSET:
            field_dict["e911_callback"] = e911_callback
        if cnam is not UNSET:
            field_dict["cnam"] = cnam
        if recording_enabled is not UNSET:
            field_dict["recording_enabled"] = recording_enabled
        if block_anonymous is not UNSET:
            field_dict["block_anonymous"] = block_anonymous
        if allow_list is not UNSET:
            field_dict["allow_list"] = allow_list
        if block_list is not UNSET:
            field_dict["block_list"] = block_list
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
        def _parse_state(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        state = _parse_state(d.pop("state", UNSET))


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


        def _parse_recording_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        recording_enabled = _parse_recording_enabled(d.pop("recording_enabled", UNSET))


        def _parse_block_anonymous(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        block_anonymous = _parse_block_anonymous(d.pop("block_anonymous", UNSET))


        def _parse_allow_list(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                allow_list_type_0 = cast(list[str], data)

                return allow_list_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        allow_list = _parse_allow_list(d.pop("allow_list", UNSET))


        def _parse_block_list(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                block_list_type_0 = cast(list[str], data)

                return block_list_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        block_list = _parse_block_list(d.pop("block_list", UNSET))


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


        update_phone_number_request = cls(
            state=state,
            e911_address=e911_address,
            e911_callback=e911_callback,
            cnam=cnam,
            recording_enabled=recording_enabled,
            block_anonymous=block_anonymous,
            allow_list=allow_list,
            block_list=block_list,
            tags=tags,
            location_id=location_id,
            site_id=site_id,
        )


        update_phone_number_request.additional_properties = d
        return update_phone_number_request

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
