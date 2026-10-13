from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="SamlLogoutPost")



@_attrs_define
class SamlLogoutPost:
    """ What a SAML provider posts to ask for a logout, or to answer FCP's
    (the HTTP-POST binding).

        Attributes:
            saml_request (None | str | Unset):
            saml_response (None | str | Unset): The provider's answer to a logout FCP asked for.
            relay_state (None | str | Unset):
     """

    saml_request: None | str | Unset = UNSET
    saml_response: None | str | Unset = UNSET
    relay_state: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        saml_request: None | str | Unset
        if isinstance(self.saml_request, Unset):
            saml_request = UNSET
        else:
            saml_request = self.saml_request

        saml_response: None | str | Unset
        if isinstance(self.saml_response, Unset):
            saml_response = UNSET
        else:
            saml_response = self.saml_response

        relay_state: None | str | Unset
        if isinstance(self.relay_state, Unset):
            relay_state = UNSET
        else:
            relay_state = self.relay_state


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if saml_request is not UNSET:
            field_dict["SAMLRequest"] = saml_request
        if saml_response is not UNSET:
            field_dict["SAMLResponse"] = saml_response
        if relay_state is not UNSET:
            field_dict["RelayState"] = relay_state

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_saml_request(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        saml_request = _parse_saml_request(d.pop("SAMLRequest", UNSET))


        def _parse_saml_response(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        saml_response = _parse_saml_response(d.pop("SAMLResponse", UNSET))


        def _parse_relay_state(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        relay_state = _parse_relay_state(d.pop("RelayState", UNSET))


        saml_logout_post = cls(
            saml_request=saml_request,
            saml_response=saml_response,
            relay_state=relay_state,
        )


        saml_logout_post.additional_properties = d
        return saml_logout_post

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
