from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.redirection_service import RedirectionService
from ..models.redirection_state import RedirectionState
from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="DeviceRedirectionResponse")



@_attrs_define
class DeviceRedirectionResponse:
    """ Where a device stands with its vendor's service.

        Attributes:
            service (RedirectionService): A vendor's redirection service: a phone out of its box asks it where
                its provisioning server is.
            mac_address (str):
            state (RedirectionState): How a device's registration with its vendor went.
            with_credentials (bool): Whether the phone was given its own credentials through the service.
            at (datetime.datetime):
            detail (None | str | Unset):
     """

    service: RedirectionService
    mac_address: str
    state: RedirectionState
    with_credentials: bool
    at: datetime.datetime
    detail: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        service = self.service.value

        mac_address = self.mac_address

        state = self.state.value

        with_credentials = self.with_credentials

        at = self.at.isoformat()

        detail: None | str | Unset
        if isinstance(self.detail, Unset):
            detail = UNSET
        else:
            detail = self.detail


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "service": service,
            "mac_address": mac_address,
            "state": state,
            "with_credentials": with_credentials,
            "at": at,
        })
        if detail is not UNSET:
            field_dict["detail"] = detail

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        service = RedirectionService(d.pop("service"))




        mac_address = d.pop("mac_address")

        state = RedirectionState(d.pop("state"))




        with_credentials = d.pop("with_credentials")

        at = datetime.datetime.fromisoformat(d.pop("at"))




        def _parse_detail(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        detail = _parse_detail(d.pop("detail", UNSET))


        device_redirection_response = cls(
            service=service,
            mac_address=mac_address,
            state=state,
            with_credentials=with_credentials,
            at=at,
            detail=detail,
        )


        device_redirection_response.additional_properties = d
        return device_redirection_response

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
